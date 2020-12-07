//! Entry point of the program.
mod error;
mod eval;
mod identifier;
mod label;
mod merge;
mod operation;
mod parser;
mod position;
mod program;
mod serialize;
mod stack;
mod stdlib;
mod term;
mod transformations;
mod typecheck;
mod types;

use crate::error::{Error, IOError, SerializationError};
use crate::label::Label;
use crate::program::Program;
use crate::term::{MergePriority, MetaValue, RichTerm, Term};
use std::io::Write;
use std::path::PathBuf;
use std::str::FromStr;
use std::{fmt, fs, io, process};
// use std::ffi::OsStr;
use structopt::StructOpt;

extern crate either;

/// Command-line options and subcommands.
#[derive(StructOpt, Debug)]
/// The interpreter of the Nickel language.
struct Opt {
    /// The input file. Standard input by default
    #[structopt(short = "f", long)]
    #[structopt(parse(from_os_str))]
    file: Option<PathBuf>,
    #[structopt(subcommand)]
    command: Option<Command>,
}

/// Available export formats.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
enum ExportFormat {
    Raw,
    Json,
}

impl std::default::Default for ExportFormat {
    fn default() -> Self {
        ExportFormat::Json
    }
}

impl fmt::Display for ExportFormat {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Raw => write!(f, "raw"),
            Self::Json => write!(f, "json"),
        }
    }
}

#[derive(Clone, Eq, PartialEq, Debug)]
pub struct ParseFormatError(String);

impl fmt::Display for ParseFormatError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "unsupported export format {}", self.0)
    }
}

impl FromStr for ExportFormat {
    type Err = ParseFormatError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_ref() {
            "raw" => Ok(ExportFormat::Raw),
            "json" => Ok(ExportFormat::Json),
            _ => Err(ParseFormatError(String::from(s))),
        }
    }
}

/// Available subcommands.
#[derive(StructOpt, Debug)]
enum Command {
    /// Export the result to a different format
    Export {
        /// Available formats: `raw, json`. Default format: `json`.
        #[structopt(long)]
        format: Option<ExportFormat>,
        /// Output file. Standard output by default
        #[structopt(short = "o", long)]
        #[structopt(parse(from_os_str))]
        output: Option<PathBuf>,
    },
    Query {
        path: Option<String>,
        #[structopt(long)]
        doc: bool,
        #[structopt(long)]
        contract: bool,
        #[structopt(long)]
        default: bool,
    },
    /// Typecheck a program, but do not run it
    Typecheck,
}

fn main() {
    let opts = Opt::from_args();
    let mut program = opts
        .file
        .map(|path: PathBuf| -> io::Result<_> {
            let file = fs::File::open(&path)?;
            Program::new_from_source(file, &path)
        })
        .unwrap_or_else(Program::new_from_stdin)
        .unwrap_or_else(|err| {
            eprintln!("Error when reading input: {}", err);
            process::exit(1)
        });

    let result = match opts.command {
        Some(Command::Export { format, output }) => export(&mut program, format, output),
        Some(Command::Query {
            path,
            doc,
            contract,
            default,
        }) => query(&mut program, path, doc, contract, default),
        Some(Command::Typecheck) => program.typecheck().map(|_| ()),
        None => program.eval().and_then(|t| {
            println!("Done: {:?}", t);
            Ok(())
        }),
    };

    if let Err(err) = result {
        program.report(err);
        process::exit(1)
    }
}

fn export(
    program: &mut Program,
    format: Option<ExportFormat>,
    output: Option<PathBuf>,
) -> Result<(), Error> {
    let rt = program.eval_full().map(RichTerm::from)?;
    serialize::validate(&rt)?;

    let format = format.unwrap_or_default();

    if let Some(file) = output {
        let mut file = fs::File::create(&file).map_err(IOError::from)?;

        match format {
            ExportFormat::Json => serde_json::to_writer_pretty(file, &rt)
                .map_err(|err| SerializationError::Other(err.to_string())),
            ExportFormat::Raw => match *rt.term {
                Term::Str(s) => file
                    .write_all(s.as_bytes())
                    .map_err(|err| SerializationError::Other(err.to_string())),
                t => Err(SerializationError::Other(format!(
                    "raw export requires a `Str`, got {}",
                    t.type_of().unwrap()
                ))),
            },
        }?
    } else {
        match format {
            ExportFormat::Json => serde_json::to_writer_pretty(io::stdout(), &rt)
                .map_err(|err| SerializationError::Other(err.to_string())),
            ExportFormat::Raw => match *rt.term {
                Term::Str(s) => std::io::stdout()
                    .write_all(s.as_bytes())
                    .map_err(|err| SerializationError::Other(err.to_string())),
                t => Err(SerializationError::Other(format!(
                    "raw export requires a `Str`, got {}",
                    t.type_of().unwrap()
                ))),
            },
        }?
    }

    Ok(())
}

fn query(
    program: &mut Program,
    path: Option<String>,
    doc: bool,
    contract: bool,
    default: bool,
) -> Result<(), Error> {
    // Print a list the fields of a term if it is a record, or do nothing otherwise.
    fn print_fields(t: &Term) {
        println!();
        match t {
            Term::Record(map) if !map.is_empty() => query::print_fields(map.keys()),
            _ => (),
        }
    }

    let all = !doc && !contract && !default;
    let term = program.eval_meta(path)?;

    match term {
        Term::MetaValue(meta) => {
            let mut found = false;
            match &meta.contract {
                Some((_, Label { types, .. })) if contract || all => {
                    query::print_metadata("contract", &format!("{}", types));
                    found = true;
                }
                _ => (),
            }

            match &meta {
                MetaValue {
                    priority: MergePriority::Default,
                    value: Some(t),
                    ..
                } if default || all => {
                    query::print_metadata("default", &t.as_ref().shallow_repr());
                    found = true;
                }
                MetaValue {
                    priority: MergePriority::Normal,
                    value: Some(t),
                    ..
                } if all => {
                    query::print_metadata("value", &t.as_ref().shallow_repr());
                    found = true;
                }
                _ => (),
            }

            match meta.doc {
                Some(s) if doc || all => {
                    query::print_metadata_doc(&s);
                    found = true;
                }
                _ => (),
            }

            if !found {
                println!("Requested metadata were not found for this value.");
                meta.value.iter().for_each(|rt| print_fields(rt.as_ref()));
            }
        }
        t => {
            println!("No metadata found for this value.");
            print_fields(&t)
        }
    }

    Ok(())
}

#[cfg(feature = "markdown")]
/// Helper to render the result of the `query` sub-command with markdown support.
mod query {
    use super::identifier::Ident;

    /// Print a metadata given as an attribute name and a value.
    pub fn print_metadata(attr: &str, value: &String) {
        use minimad::*;
        use termimad::*;

        let skin = mk_skin();
        let mut expander = OwningTemplateExpander::new();
        let template = TextTemplate::from("* **${attr}**: *${value}*");

        expander.set("attr", attr);
        expander.set("value", value);
        let text = expander.expand(&template);
        let (width, _) = terminal_size();
        let fmt_text = FmtText::from_text(&skin, text, Some(width as usize));
        print!("{}", fmt_text);
    }

    /// Print the documentation included in a metavalue.
    pub fn print_metadata_doc(content: &String) {
        let skin = mk_skin();

        if content.find("\n").is_none() {
            skin.print_text(&format!("* **documentation**: {}", content));
        } else {
            skin.print_text("* **documentation**\n\n");
            skin.print_text(content);
        }
    }

    /// Create the renderer configuration.
    fn mk_skin() -> termimad::MadSkin {
        use termimad::MadSkin;
        MadSkin::default()
    }

    /// Print the list of fields of a record.
    pub fn print_fields<'a, I>(fields: I)
    where
        I: Iterator<Item = &'a Ident>,
    {
        use minimad::*;
        use termimad::*;

        let skin = mk_skin();
        let (width, _) = terminal_size();
        let mut expander = OwningTemplateExpander::new();
        let template = TextTemplate::from("* ${field}");

        skin.print_text("## Available fields");

        for field in fields {
            expander.set("field", field.to_string());
            let text = expander.expand(&template);
            let fmt_text = FmtText::from_text(&skin, text, Some(width as usize));
            print!("{}", fmt_text);
        }
    }
}

#[cfg(not(feature = "markdown"))]
/// Helper to render the result of the `query` sub-command without markdown support.
mod query {
    use super::identifier::Ident;

    /// Print a metadata given as an attribute name and a value.
    pub fn print_metadata(name: &str, content: &String) {
        println!("* {}: {}", name, content);
    }

    /// Print the documentation included in a metavalue.
    pub fn print_metadata_doc(content: &String) {
        if content.find("\n").is_none() {
            print_metadata("documentation", &content);
        } else {
            println!("* documentation\n");
            println!("{}", content);
        }
    }

    /// Print the list of fields of a record.
    pub fn print_fields<'a, I>(fields: I)
    where
        I: Iterator<Item = &'a Ident>,
    {
        println!("Available fields:");

        for field in fields {
            println!(" - {}", field);
        }
    }
}
