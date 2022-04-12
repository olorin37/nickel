use criterion::{criterion_main, Criterion};
use nickel_lang_utilities::ncl_bench_group;
use pprof::criterion::{Output, PProfProfiler};

ncl_bench_group! {
    name = benches;
    config = Criterion::default().with_profiler(PProfProfiler::new(100, Output::Flamegraph(None)));
    {
        name = "round_trip",
        path = "serialization/main",
        subtest = "input",
    }
}
criterion_main!(benches);
