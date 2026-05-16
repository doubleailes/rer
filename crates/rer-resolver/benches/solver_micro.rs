//! Solver hot-path micro-benchmarks.
//!
//! These exist to catch regressions in operations we've spent PRs
//! optimising (`PackageVariantSlice::{intersect, reduce_by, extract}`,
//! `Requirement::conflicts_with`, `VersionRange::{union, intersection,
//! intersects}`, plus an end-to-end small solve). They use a hand-rolled
//! synthetic repo — small, deterministic, no fixture file dependency — so the
//! bench is reproducible on any checkout without running
//! `scripts/prepare_benchmark_data.py`.
//!
//! Suggested workflow:
//!
//! ```text
//! # On main: capture a baseline.
//! cargo bench -p rer-resolver --bench solver_micro -- --save-baseline main
//!
//! # On a perf branch: compare against it.
//! cargo bench -p rer-resolver --bench solver_micro -- --baseline main
//! ```
//!
//! The macro 188-case benchmark (`examples/rez_benchmark_dataset`) remains
//! the canonical end-to-end perf signal; this file complements it with
//! stable, sub-millisecond unit-level numbers.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use rer_resolver::rez_solver::{
    make_shared_cache, PackageVariantSlice, Requirement, RequirementList, Solver, SolverContext,
    SolverStatus,
};
use rer_resolver::PackageData;
use rer_version::VersionRange;
use std::collections::HashMap;
use std::rc::Rc;

// ---------------------------------------------------------------------------
// Synthetic repo
// ---------------------------------------------------------------------------

/// Build the bench's fixed repo: six common-shaped families covering the
/// patterns the solver hot-path actually walks (multi-version, multi-variant,
/// shared dependencies). Stable inputs → stable baselines.
fn build_repo() -> HashMap<String, HashMap<String, PackageData>> {
    let mut repo: HashMap<String, HashMap<String, PackageData>> = HashMap::new();

    // python — 6 versions, no deps, no variants.
    fam(
        &mut repo,
        "python",
        &[
            ("3.7.0", &[], &[]),
            ("3.8.0", &[], &[]),
            ("3.9.0", &[], &[]),
            ("3.10.0", &[], &[]),
            ("3.11.0", &[], &[]),
            ("3.12.0", &[], &[]),
        ],
    );

    // qt — 3 versions, no deps.
    fam(
        &mut repo,
        "qt",
        &[
            ("5.15.0", &[], &[]),
            ("6.0.0", &[], &[]),
            ("6.5.0", &[], &[]),
        ],
    );

    // maya — 4 versions, two variants each pinning a python.
    fam(
        &mut repo,
        "maya",
        &[
            ("2022.0", &[], &[&["python-3.7"], &["python-3.8"]]),
            ("2023.0", &[], &[&["python-3.9"], &["python-3.10"]]),
            ("2024.0", &[], &[&["python-3.10"], &["python-3.11"]]),
            ("2025.0", &[], &[&["python-3.11"], &["python-3.12"]]),
        ],
    );

    // nuke — variants share python with maya.
    fam(
        &mut repo,
        "nuke",
        &[
            ("14.0", &[], &[&["python-3.9"], &["python-3.10"]]),
            ("15.0", &[], &[&["python-3.10"], &["python-3.11"]]),
            ("15.1", &[], &[&["python-3.11"], &["python-3.12"]]),
        ],
    );

    // usd — base requires python + qt, no variants.
    fam(
        &mut repo,
        "usd",
        &[
            ("23.05", &["python-3.10", "qt-5"], &[]),
            ("23.11", &["python-3.11", "qt-5"], &[]),
            ("24.05", &["python-3.11", "qt-6"], &[]),
        ],
    );

    // app — composes maya + nuke + usd.
    fam(
        &mut repo,
        "app",
        &[
            ("1.0.0", &["maya-2024", "nuke-15", "usd"], &[]),
            ("2.0.0", &["maya-2025", "nuke-15.1", "usd-24"], &[]),
        ],
    );

    repo
}

/// Insert a family + versions into the repo. `versions` is a slice of
/// `(version_str, requires, variants)`.
fn fam(
    repo: &mut HashMap<String, HashMap<String, PackageData>>,
    name: &str,
    versions: &[(&str, &[&str], &[&[&str]])],
) {
    let mut by_version: HashMap<String, PackageData> = HashMap::new();
    for (v, requires, variants) in versions {
        by_version.insert(
            (*v).to_string(),
            PackageData {
                requires: requires.iter().map(|s| (*s).to_string()).collect(),
                variants: variants
                    .iter()
                    .map(|var| var.iter().map(|s| (*s).to_string()).collect())
                    .collect(),
            },
        );
    }
    repo.insert(name.to_string(), by_version);
}

/// Build a single `PackageVariantSlice` for `family` over `range`, going
/// through the same cache code path the solver uses. Used by the slice-level
/// benches below.
fn slice_for(family: &str, range: &VersionRange) -> PackageVariantSlice {
    let repo = Rc::new(rer_resolver::rez_solver::PackageRepo::from_map(build_repo()));
    let ctx = Rc::new(SolverContext::new(repo, RequirementList::new(vec![])));
    ctx.get_variant_slice(family, range)
        .expect("slice for the requested family/range")
}

// ---------------------------------------------------------------------------
// VersionRange micro-benches
// ---------------------------------------------------------------------------

fn bench_version_range(c: &mut Criterion) {
    let mut group = c.benchmark_group("VersionRange");

    let cases: &[(&str, &str, &str)] = &[
        ("disjoint", "1.0..2.0", "3.0..4.0"),
        ("overlap", "1.0..2.5", "2.0..3.0"),
        ("subset", "1.0..3.0", "1.5..2.5"),
        ("any-vs-narrow", "", "2.0..3.0"),
    ];

    for (label, a_s, b_s) in cases {
        let a = VersionRange::parse(a_s);
        let b = VersionRange::parse(b_s);

        group.bench_with_input(BenchmarkId::new("union", label), &(&a, &b), |bn, (a, b)| {
            bn.iter(|| black_box(a).union(black_box(b)));
        });
        group.bench_with_input(
            BenchmarkId::new("intersection", label),
            &(&a, &b),
            |bn, (a, b)| {
                bn.iter(|| black_box(a).intersection(black_box(b)));
            },
        );
        group.bench_with_input(
            BenchmarkId::new("intersects", label),
            &(&a, &b),
            |bn, (a, b)| {
                bn.iter(|| black_box(a).intersects(black_box(b)));
            },
        );
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Requirement micro-benches
// ---------------------------------------------------------------------------

fn bench_requirement(c: &mut Criterion) {
    let mut group = c.benchmark_group("Requirement");

    // `parse` exercises the thread-local memo plus the actual parser. We
    // hit a fresh string each iteration to also measure the cache lookup.
    group.bench_function("parse(unique)", |b| {
        let mut counter = 0u64;
        b.iter(|| {
            counter = counter.wrapping_add(1);
            let s = format!("python-{counter}");
            black_box(Requirement::parse(&s))
        });
    });
    group.bench_function("parse(memoised)", |b| {
        b.iter(|| black_box(Requirement::parse("python-3.10")));
    });

    let a = Requirement::parse("python-3.10");
    let b_no = Requirement::parse("python-3.10.4");
    let b_yes = Requirement::parse("python-3.11");
    let unrelated = Requirement::parse("maya-2024");

    group.bench_function("conflicts_with(compatible)", |bn| {
        bn.iter(|| black_box(&a).conflicts_with(black_box(&b_no)));
    });
    group.bench_function("conflicts_with(conflict)", |bn| {
        bn.iter(|| black_box(&a).conflicts_with(black_box(&b_yes)));
    });
    group.bench_function("conflicts_with(other-family)", |bn| {
        bn.iter(|| black_box(&a).conflicts_with(black_box(&unrelated)));
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// PackageVariantSlice micro-benches
// ---------------------------------------------------------------------------

fn bench_slice(c: &mut Criterion) {
    let mut group = c.benchmark_group("PackageVariantSlice");

    let maya = slice_for("maya", &VersionRange::any());
    let any = VersionRange::any();
    let narrow = VersionRange::parse("2024+");
    let drop_all = VersionRange::parse("9000+");

    group.bench_function("intersect(any → Unchanged)", |b| {
        b.iter(|| black_box(&maya).intersect(black_box(&any)));
    });
    group.bench_function("intersect(narrow → Narrowed)", |b| {
        b.iter(|| black_box(&maya).intersect(black_box(&narrow)));
    });
    group.bench_function("intersect(empty → Empty)", |b| {
        b.iter(|| black_box(&maya).intersect(black_box(&drop_all)));
    });

    // reduce_by: family that *isn't* in fam_requires hits the fast-path
    // (one HashSet lookup, returns Unchanged) — 98 % of calls in the
    // benchmark take this path, so a regression here is extra costly.
    let qt = Requirement::parse("qt-6");
    // family that *is* in maya's fam_requires (python) — exercises the
    // full reduce loop, conflict_tests cache, the lot.
    let python_compat = Requirement::parse("python-3.10");
    let python_conflict = Requirement::parse("python-3.7");

    group.bench_function("reduce_by(fast-path, unrelated fam)", |b| {
        b.iter(|| black_box(&maya).reduce_by(black_box(&qt)));
    });
    group.bench_function("reduce_by(full body, no reductions)", |b| {
        b.iter(|| black_box(&maya).reduce_by(black_box(&python_compat)));
    });
    group.bench_function("reduce_by(full body, with reductions)", |b| {
        b.iter(|| black_box(&maya).reduce_by(black_box(&python_conflict)));
    });

    group.bench_function("extract(common dep)", |b| {
        // A fresh slice on every iteration — `extract` mutates extracted_fams
        // internally, so we need a clean slice each time to measure the cost
        // of an actual extraction (rather than the cheap "nothing left"
        // early-return).
        b.iter_batched(
            || slice_for("maya", &VersionRange::any()),
            |s| black_box(&s).extract(),
            criterion::BatchSize::SmallInput,
        );
    });
    group.bench_function("extract(exhausted → None)", |b| {
        // Already-extracted slice (extracted_fams == common_fams). Measures
        // the O(1) length-compare early-return path from PR #71.
        let exhausted = {
            let mut s = slice_for("maya", &VersionRange::any());
            while let Some((next, _req)) = s.extract() {
                s = next;
            }
            s
        };
        b.iter(|| black_box(&exhausted).extract());
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// End-to-end small solve
// ---------------------------------------------------------------------------

fn bench_solve(c: &mut Criterion) {
    let mut group = c.benchmark_group("Solver");
    let repo = Rc::new(rer_resolver::rez_solver::PackageRepo::from_map(build_repo()));
    let cache = make_shared_cache();

    let cases: &[(&str, &[&str])] = &[
        ("single", &["maya-2024"]),
        ("pair", &["maya-2024", "nuke-15"]),
        ("triple-with-pin", &["maya", "nuke", "python-3.11"]),
        ("app", &["app-1"]),
    ];

    for (label, reqs) in cases {
        let parsed: Vec<Requirement> = reqs.iter().map(|s| Requirement::parse(s)).collect();
        group.bench_function(*label, |b| {
            b.iter_batched(
                || (parsed.clone(), Rc::clone(&repo), cache.clone()),
                |(reqs, repo, cache)| {
                    let mut solver = Solver::new_with_cache(reqs, repo, cache).unwrap();
                    solver.solve();
                    assert_eq!(solver.status(), SolverStatus::Solved);
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

criterion_group!(
    name = benches;
    config = Criterion::default();
    targets = bench_version_range, bench_requirement, bench_slice, bench_solve,
);
criterion_main!(benches);
