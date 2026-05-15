+++
title = "Why rer doesn't cache intersect / reduce_by results"
description = "rez carries a per-slice dedup cache on its solver hot path; porting it to rer is a 2x regression. Here is the data, the analysis, and the decision."
draft = false
weight = 10
sort_by = "weight"
template = "docs/page.html"

[extra]
lead = "rez carries a per-slice dedup cache on its solver hot path. Porting it to rer is a 2× regression, not a speedup. Here is the data, the analysis, and the decision."
toc = true
top = false
+++

## Background

rez's package solver (`rez/src/rez/solver.py`) maintains two per-slice
caches on `_PackageVariantSlice` that are documented in `SOLVER.md`:

- **`been_intersected_with`** — the set of version ranges this slice
  has already been narrowed by. On a repeat `intersect(R)` call, the
  slice is already a subset of `R`, so the function returns
  `Unchanged` without iterating its entries.
- **`been_reduced_by`** — the set of package requests this slice has
  already been filtered against. On a repeat `reduce_by(P)` call, no
  variant can newly conflict with `P` (because narrowing only ever
  removes variants), so the function returns `Unchanged` without
  iterating.

In rez, these short-circuit paths are part of an "optimised mode" and
contribute meaningfully to overall solve time. When rer's solver was
audited against `SOLVER.md` (issue [#64][issue-64]), the absence of
these caches was flagged as a likely performance gap to close.

The audit was wrong. The data showed that adding them is **a net
regression on rer** — not a small one, and not specific to a corner
case. This page records what happened, the measurements, and why the
optimisation doesn't translate.

## The experiment

The implementation followed rez's reference closely:

- Added `been_intersected_with: RefCell<FxHashSet<VersionRange>>` and
  `been_reduced_by: RefCell<FxHashSet<Requirement>>` to
  `PackageVariantSlice`.
- Early-return on cache hit in `intersect` / `reduce_by`.
- Propagated the caches in `copy_with_entries`, mirroring
  `_PackageVariantSlice._copy` in `solver.py:861-869`. The invariant
  "every entry lies in range R" and "no variant conflicts with
  request P" are both preserved under further entry-narrowing, so
  the cache stays valid when the slice shrinks.
- Added thread-local counters around the cache check to measure
  call counts and hit rates.
- Tried a "cheap key" variant on top: memoised the `Hash` output on
  `VersionRange` via an internal `OnceCell<u64>`, so each repeated
  lookup pays a cell read rather than walking every segment of the
  range.

Each variant was measured against rez's bundled 188-case benchmark
in release mode, on the same machine, with all caches warm.

Correctness was checked on every variant: the 188 cases continued to
match rez's recorded resolutions 1:1, so the cache is semantically
sound. It just doesn't help.

## Results

Five variants, two baselines, one verdict:

| Variant                                       |    Total | Mean   | Δ vs baseline |
|-----------------------------------------------|---------:|-------:|--------------:|
| **Baseline (main, no dedup cache)**           |  43.0 s  | 230 ms | —             |
| `derive(Hash)` + intersect cache only         |  44.1 s  | 235 ms | +2.6 %        |
| `derive(Hash)` + both caches (rez parity)     |  86.3 s  | 459 ms | **+101 %**    |
| Memoised hash + both caches                   | 132.1 s  | 703 ms | **+207 %**    |
| Memoised hash + intersect cache only          |  46.1 s  | 245 ms | +7.2 %        |

Every variant is worse than baseline. The straight port of rez's
optimisation is twice as slow. The "obvious" improvement
(memoise the hash) makes it three times as slow — the memoisation
costs more than it saves.

## Why: instrumentation tells the story

Counters around the two operations, summed across all 188 cases:

| Operation     |       Calls |    Hits | Hit rate |
|---------------|------------:|--------:|---------:|
| `intersect`   |     288,729 | 206,211 |  **71.4 %**   |
| `reduce_by`   |  26,980,353 |  78,368 |   **0.29 %**  |

Two separate problems:

**`reduce_by` is called twenty-seven million times.** The cache lookup
fires on every one of them, but lands less than a third of one percent
of the time. We pay the per-call hash cost on 26.9M misses to save the
filter work on 78k hits. The cache is essentially pure overhead on
this path.

**`intersect` hits land 71% of the time.** That sounds like it should
help — and in Python, it does. But the work being skipped is a tight
`entries.iter().filter` over `Rc`-shared entries with `range.contains`
per entry: roughly 100 nanoseconds for a typical slice. The cache
check (hash a `VersionRange`, look it up in a `FxHashSet`) costs the
same order of magnitude. With a 71% hit rate, the average per-call
cost is approximately equal to baseline, which is exactly what the
measurements show: the intersect-only variant is within noise of
main (44.1 s vs 43.0 s).

## Why the memoised-hash variant is worse, not better

The natural follow-up hypothesis: the cache is good, the hash function
is too slow, fix the hash. Memoise the `Hash` output on `VersionRange`
via an internal `OnceCell<u64>` so the second time a given range is
hashed it's a single cell read.

This made things worse. Two reasons:

1. `VersionRange` is cloned heavily in the solver — in `union`,
   `intersection`, `from_versions`, `from_version`, and as part of
   every cloned `Requirement`. Each clone carries the `OnceCell<u64>`
   field. The per-clone memory and copy cost adds up across millions
   of clones.

2. The first call to `Hash::hash` on a freshly cloned `VersionRange`
   pays the segment walk *plus* a `OnceCell::set`. For ranges that
   are hashed only once or twice (which most are, given the call
   distribution), memoisation is a net loss before it ever pays off.

So even the intersect-only variant gets *worse* with memoised hashing
(46.1 s vs 44.1 s without memoisation).

## Root cause: the cost ratio is inverted between Python and Rust

rez's optimisation is correct and meaningful — in Python. The reason
it works there but not here is that the *ratio* between two
operations is flipped:

|                                | rez (Python)          | rer (Rust)                |
|--------------------------------|-----------------------|---------------------------|
| Cost of "redo the work" (`intersect`/`reduce_by` body) | Function-call dispatch, list comprehension, attribute lookup per variant — **dominant cost** | Tight loop over `Rc<PackageEntry>` with `range.contains` per entry — **cheap** |
| Cost of "check the cache" (`x in set`) | Hash + dict probe — **cheap relative to the body** | Hash a `VersionRange` (walk segments) + `FxHashSet::contains` — **comparable to the body** |

A cache pays off when "check the cache" is cheap relative to "do the
work". rez's interpreter overhead inflates the work side; rer's
release-mode Rust doesn't.

There's no clever variation of this cache that fixes the cost ratio
without a much bigger restructuring (e.g. interning every
`VersionRange` and `Requirement` and keying by pointer identity).
Even a hypothetical zero-cost cache would only recover the ~1 second
the intersect-only variant currently loses to overhead — that's the
ceiling, and it's not worth the surface area.

## Decision

The dedup caches are **not implemented** in rer. The 188-case
benchmark already shows rer at 8.8× rez on the same machine; the
remaining headroom is elsewhere (extraction, splitting, requirement
parsing — not the slice short-circuits).

Issue [#64][issue-64] is closed as `not planned`. The full data,
including raw measurements and per-variant cache hit rates, is
attached to that issue.

This page exists so that the next time someone reads `SOLVER.md` and
thinks "rer is missing an optimisation," they find the answer here
instead of redoing the experiment.

## What this says more generally

Optimisations don't port across languages on their own. They embed an
assumption about the cost of each primitive in the algorithm, and
when the primitives change cost, the optimisation can flip from
helpful to harmful. The right way to evaluate a port of a Python
optimisation to Rust is to measure both sides — never assume.

[issue-64]: https://github.com/doubleailes/rer/issues/64
