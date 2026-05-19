+++
title = "Road to 1.0.0 — release checklist"
description = "Concrete checklist for cutting a stable 1.0.0 from the 1.0.0-rc.X line. API surface review, correctness hardening, docs polish, release pipeline."
draft = false
weight = 25
sort_by = "weight"
template = "docs/page.html"

[extra]
lead = "Concrete checklist for cutting a stable 1.0.0 from the 1.0.0-rc.X line. The point of 1.0 isn't more features — it's freezing what we have so callers can rely on it."
toc = true
top = false
+++

> **Companion to the [Stability commitments](../stability/) page.**
> That page describes what 1.0 commits us to. This page describes
> the work we need to do *before* cutting 1.0 so the commitment is
> defensible. Read both together.

The 1.0.0-rc.X line ships everything we want feature-wise. The
remaining work is *deciding what's `pub`*, *proving it's correct*,
and *announcing what we promise*. None of it adds new functionality.
All of it is small PRs.

## Why a 1.0 cut isn't "ship rc.3 with a different version number"

Once 1.0.0 lands:

- Every public type / function / behaviour is under semver.
  Breaking it requires a major-version bump.
- The integration shim authors (Fortiche, anyone else) take the
  shape we ship as load-bearing forever. Any wart becomes
  permanent.
- "It's documented as reserved-and-ignored" stops being a get-out-
  of-jail-free card; reserved-and-ignored kwargs become API noise
  we can't drop.

So the bar isn't more features — it's confidence we can support
what's already shipped without painting ourselves into a corner.

## Must-have work

These items affect the public surface area we commit to. Doing
them before 1.0 is much cheaper than doing them after (where they
become 2.0 breaking changes with deprecation cycles).

### 1. `pyrer.solve()` signature cleanup

Today:

```python
pyrer.solve(package_requests, packages=None, /, *, load_family=None,
            variant_select_mode="version_priority",
            filters=None, max_iterations=None)
```

Decisions:

- **`filters` parameter.** Reserved-but-ignored. **Drop** unless
  we plan to ship the implementation in 1.0. Reserved kwargs that
  outlive a major version become permanent noise; reintroducing
  later is cheap, removing later is not.
- **`max_iterations` parameter.** Same. **Drop.**
- **`load_family` 1-arg vs 2-arg signature.** Today pyrer auto-
  detects via `inspect.signature` and dispatches accordingly.
  **Decide**: commit to keeping the 1-arg form forever (no
  change), or add a `DeprecationWarning` when a 1-arg callback is
  detected and drop in 2.0 (slightly cleaner long-term).
- **`SolveResult.resolved` tuple form.** Documented as "kept for
  compatibility with the original 0.1.0-rc.5 surface". **Decide**:
  mark deprecated for 2.0 removal (recommended — `resolved_packages`
  covers every use), or commit to permanent.

### 2. Rust API audit per crate

Walk each crate and mark anything internal as `pub(crate)`. The
exposed surface for `rer-resolver` today is large; some items are
implementation detail callers shouldn't depend on.

- **`rer-version`** — `RerVersion`, `VersionRange`,
  `Requirement`, `Requirements`. Audit each.
- **`rer-resolver`** — every `pub use` in `rez_solver/mod.rs`.
  Items like `PackageVariantCache`, `SolverContext`,
  `PackageScope`, `ResolvePhase` are arguably internal-only;
  decide which the outside world needs.
- **`rer-package`** — `PackageInfo`, `parse_static_package_py`,
  `parse_static_packages_py`. All seem genuinely public.

For every `pub` item that stays, decide whether it's a stable
contract or experimental.

### 3. Public enums need `#[non_exhaustive]`

If we add a variant to `FailureReason` / `ScopeError` /
`SolverStatus` / `VariantSelectMode` in 1.1, that's a non-breaking
change *only if* the enum is `#[non_exhaustive]` so callers
already use `_ =>` arms. Add the attribute to every public enum
before 1.0.

### 4. `FamilyLoader` type — hide or commit?

The closure type leaks `&VersionRange` from `rer-version`, which
means external implementors of `FamilyLoader` depend on that
crate. Two options:

- **Commit** to the current shape and add `rer-version` as a
  required dependency for `FamilyLoader` users.
- **Hide** by changing the loader signature to accept a `&str`
  (rez range syntax) — pyrer already converts to/from strings at
  the FFI boundary.

The second is more contained. Recommend it.

## Should-have work

Items that strengthen the release without changing the API.

### 5. Real-corpus differential in CI

The 188-case `test_rez_benchmark` is solid but issue #96 (even
when downstream-attributable) demonstrated it doesn't exercise
studio shapes:

- Packages with 4+ variants each disagreeing on python version.
- Bare `requires` on long lists of internal families.
- Transitive variant requirements forcing deep backtracking.

Plan:

1. Extract ~200 packages from a representative real corpus
   (anonymized — strip `commands()` content, replace internal
   names with generic stand-ins) into a committed fixture in
   `data_set/real_corpus/`.
2. Wire `scripts/compare_resolves.py` into CI as a
   `cargo test --ignored`-style gate against that fixture.
3. Any divergence is a release blocker, mirroring the existing
   188-case bench's role.

Effort: ~1 week. Catches future regressions on the long-tail
patterns the 188-case bench misses.

### 6. Property / fuzz test on the solver

Random variant trees and requests, run both pyrer (always) and
rez (when available), assert agreement. Codifies the
backtracking-completeness claim against synthetic inputs.

Effort: ~1–2 weeks. Catches algorithmic regressions that
hand-crafted tests don't cover.

### 7. Stability commitments page audit

The page currently lists qualitative commitments. For 1.0, name
the exact items:

- Python API: every public function on `pyrer`, every public
  field on `PackageData` / `ResolvedVariant` / `SolveResult`.
- Rust API: every `pub use` in each crate's top-level `lib.rs`,
  with a "stable" / "experimental" tag per item.
- Behavioural commitments: rez differential success criteria,
  semver policy for solver result shape, error variants.

Effort: ~3 days. Comes naturally out of the surface audit (item 2).

### 8. README polish

- Drop the "experimental" / "RC" framing where it still appears.
- Update headline numbers (34× solver / 34.8× parser / 2.81×
  batched) with their corpus context.
- Add an honest "what rer is not" section.

Effort: ~1 day.

### 9. Issue triage

Run `gh issue list --state open --repo doubleailes/rer`. Per
issue:

- **#96** — confirm with the user that the bisect tool resolves
  their concern (shim-attributable), or get a repro.
- Any "would be nice for 1.0" issue — explicit decision: in or
  out. Default to "out" unless it changes public API.
- Anything tagged as a 1.0 blocker — resolve or document the
  punt.

## Release pipeline (must work before tagging 1.0.0)

### 10. Wheel publishing for `pyrer`

- Maturin builds for Linux / macOS / Windows on cp39+ ABI3.
- CI job triggered on `v*` tags publishes to PyPI.
- Verify on a real machine: `pip install pyrer==1.0.0` produces
  a working install with the expected behaviour.

Probably already exists from the rc cycle — verify the 1.0 tag
triggers it cleanly.

### 11. crates.io publishing for the Rust crates

Order matters because of inter-crate deps:

1. `rer-version` (no internal deps).
2. `rer-resolver` (depends on `rer-version`).
3. `rer-package` (independent except for workspace metadata).

`rer-python` stays `publish = false` — it ships via PyPI as
`pyrer`.

For each: `cargo publish --dry-run` first, then real publish on
tag.

### 12. Doc deployment

Zola site rebuilds on tag → GitHub Pages. Verify the deployment
workflow exists and runs cleanly. Update the homepage version
pill (`docs/config.toml`, `docs/content/_index.md`) — same four
touchpoints as every prior rc bump.

### 13. GitHub release

- Tag `v1.0.0`.
- Release page with the `[1.0.0]` changelog excerpt as the body.
- Attach the platform wheels (or link to PyPI).
- Announce: discussions thread / Discord / wherever the
  community lives.

## Suggested sequence

Four discrete PRs, each ~3-4 days of focused work. Each can be
merged independently; ordering matters only for the API audit
(item 2) which constrains the others.

### Week 1 — API surface freeze

PR: drop `filters` / `max_iterations`; deprecate
`SolveResult.resolved`; audit Rust `pub` items; add
`#[non_exhaustive]` to public enums; decide the
`FamilyLoader`-string question; update integration docs.

### Week 2 — Correctness net

PR: extract fixture corpus into `data_set/real_corpus/`; wire
`compare_resolves.py` into CI; scaffold the property test
generator; triage open issues.

### Week 3 — Documentation

PR: stability commitments page lists the exact public surface;
README updated; changelog `[1.0.0-rc.3]` retitled to
`[1.0.0] — <date>`; migration guide (if any deprecations from
Week 1 need one).

### Week 4 — Release

PR: bump workspace + docs to `1.0.0`; verify pipelines; tag;
publish wheels; publish crates; announce.

## What's *not* in 1.0

Resist scope creep. These belong in 1.1+, not in the 1.0 cut:

- **Rex evaluator** — 2-3 month project, paired safety net required.
  Worth shipping; not worth blocking 1.0 for.
- **Persistent caches** (parsed-package, solve-result) — orthogonal
  to API stability. Can ship as feature work in 1.1.
- **Parallel `solve_many`** — needs an `Rc → Arc` refactor with
  measurable single-resolve perf risk. Not part of the 1.0 API.
- **Daemon model** — wrong scope for "rer is the engine".
- **rez-equivalent features** (CLI, build system, plugin host) —
  out of scope per the engine-only project mission.

The 1.0 cut should be "everything we have now, with the API
frozen, the docs sharpened, and the release pipeline proven".
The bar isn't more features; it's confidence we can support what's
already shipped.

## When to actually cut

Sign-off conditions:

1. Public API audit complete and committed.
2. `cargo test` + `pytest tests/` + 188-case differential +
   real-corpus differential all pass on `main`.
3. Stability commitments page lists every public symbol.
4. README reflects shipped behaviour, no "experimental" framing.
5. At least one downstream consumer (Fortiche shim) has run
   against the post-Week-1 API and confirmed it still works.
6. Open issues that affect 1.0 are resolved or explicitly punted.

If any of those isn't true, ship another rc instead.
