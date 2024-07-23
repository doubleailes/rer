use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use rer_version::{Requirement, RerVersion};

fn bench_requirement(c: &mut Criterion) {
    let mut group = c.benchmark_group("Requirement");
    group.bench_with_input(
        BenchmarkId::new("requierement_from_str", "superset"),
        "maya-1.2",
        |b, i| {
            b.iter(|| Requirement::from(i));
        },
    );
    group.bench_with_input(
        BenchmarkId::new("requierement_from_str", "exact version"),
        "maya==1.2.0",
        |b, i| {
            b.iter(|| Requirement::from(i));
        },
    );
    group.bench_with_input(
        BenchmarkId::new("requierement_from_str", "inclusive bound"),
        "maya-1.0.0..2.0.0",
        |b, i| {
            b.iter(|| Requirement::from(i));
        },
    );
    group.bench_with_input(
        BenchmarkId::new("requierement_from_str", "lower bound"),
        "maya-1.0.0+",
        |b, i| {
            b.iter(|| Requirement::from(i));
        },
    );
    group.bench_with_input(
        BenchmarkId::new("requierement_from_str", "upper bound"),
        "maya<=1.0.0",
        |b, i| {
            b.iter(|| Requirement::from(i));
        },
    );
    group.bench_with_input(
        BenchmarkId::new("requierement_from_str", "ascending order"),
        "maya-1.0.0+<2.0.0",
        |b, i| {
            b.iter(|| Requirement::from(i));
        },
    );
    group.bench_with_input(
        BenchmarkId::new("requierement_from_str", "descending order"),
        "maya<=2.0.0,1.0.0+",
        |b, i| {
            b.iter(|| Requirement::from(i));
        },
    );
    group.finish();
}

fn bench_version(c: &mut Criterion) {
    let mut group = c.benchmark_group("RerVersion");
    for version in ["1.2.3", "1.0.0-alpha", "2023.01.01", "0.0.1-beta"] {
        group.bench_with_input(
            BenchmarkId::new("rer_version_from_str", version),
            version,
            |b, _i| {
                b.iter(|| RerVersion::try_from(version));
            },
        );
    }

    group.finish();
}
criterion_group!(name = benches;config = Criterion::default(); targets=bench_requirement,bench_version);
criterion_main!(benches);
