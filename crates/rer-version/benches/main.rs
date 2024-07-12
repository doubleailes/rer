use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use rer_version::parser::VersionParsed;

fn bench_parser(c: &mut Criterion) {
    let mut group = c.benchmark_group("Parser");
    group.bench_with_input(BenchmarkId::new("from_str", "version"), "1.2.3", |b, i| {
        b.iter(|| VersionParsed::parse_str(i));
    });
    group.finish();
}
criterion_group!(name = benches;config = Criterion::default(); targets=bench_parser);
criterion_main!(benches);
