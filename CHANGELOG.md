# Changelog

All notable changes to `rer` (the Rust workspace) and `pyrer` (the PyPI
distribution of the Python bridge) are listed here.

The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). Stability
guarantees that take effect from 1.0.0 onwards are documented in the
[Stability commitments](https://doubleailes.github.io/rer/docs/engineering/stability/)
page.

## [Unreleased]

### Added

- **`version_range` hint on the `load_family` callback** (issue #92) —
  `pyrer.solve(..., load_family=cb)` now invokes `cb` as
  `cb(name, version_range="2+<3")` when the callback's signature can
  accept a second `version_range` argument (named param or `**kwargs`).
  The hint is a rez-syntax range string the shim can pass directly to
  `rez.packages.iter_packages(range_=...)` to skip on-disk version
  directories outside the request. Backward-compatible: 1-arg
  callbacks (`def cb(name):`) keep working unchanged — pyrer detects
  the signature via `inspect.signature` once per `solve()` call.
  Targets the 95% load-fan-out waste documented in #92 (2,637
  packages loaded for 132 used on a typical Fortiche resolve);
  projected 6-20× cut to `_load_family` wall time. The 188-case rez
  differential still passes 188/188.
- **`PackageData.from_strings(name, version, requires=None, variants=None)`** —
  classmethod constructor for raw-string callers, symmetric with
  `from_rez(pkg)`. Skips rez's `AttributeForwardMeta` chain, the
  `Requirement` parse, and the `str(Requirement)` round-trip — the
  latter being a measurable fraction of integration overhead on
  rez-shim hot paths (per-package, every package, every resolve).
  Functionally equivalent to the four-arg constructor; the
  classmethod form exists so callers wiring `pkg.resource.data`
  through pyrer have a named, documented contract to reach for.
  Falls back to `from_rez` for `@early` / `@late`-bound attributes.
  Closes #88.
- **`load_family` callback** on `pyrer.solve()` — opt-in lazy package
  discovery: pass `load_family: Callable[[str], list[PackageData]]` and the
  solver calls it on demand the first time it needs a family it hasn't seen.
  Each family is loaded at most once per solve; returning `[]` means "no
  such family". Aimed at cold-cache / network-filesystem integrations
  (Windows + CIFS in particular) where the up-front BFS of every reachable
  family dominates the wall-clock cost of `rez env`. See the
  [lazy-discovery section of the rez integration page](https://doubleailes.github.io/rer/docs/getting-started/rez-integration/#lazy-package-discovery-on-cold-caches).
  Closes #86.
- **`resolved_ephemerals`** on `pyrer.SolveResult` — list of rez-style
  ephemeral requirement strings (e.g. `[".feature-1.5", ".mode-debug"]`)
  surfaced from the solver, matching `rez.solver.Solver.resolved_ephemerals`.
  Closes #84.
- **Borrowing-iterator forms** on the Rust API: `Solver::resolved_packages_iter`
  / `resolved_ephemerals_iter` and `ResolvePhase::iter_solved_variants` /
  `iter_solved_ephemerals`. Avoid the intermediate `Vec` (and, for
  ephemerals, the per-element `Requirement::clone`) when callers just want
  to iterate.

### Changed

- **`PackageRepo` is now a struct**, not a `HashMap` type alias. Carries a
  cache (`RefCell<HashMap<…>>`) and an optional `FamilyLoader` closure.
  Construct with `PackageRepo::from_map(map)` for the eager case, or
  `PackageRepo::with_loader(loader)` for lazy. `From<HashMap<…>>` is
  implemented for back-compat with the old type-alias shape. The eager
  path's perf is unchanged in measurement (within run-to-run noise of the
  README baseline).

## [1.0.0] — TBD

The first stable release. Public API is now under semver — see the
[Stability commitments](https://doubleailes.github.io/rer/docs/engineering/stability/)
page for what's covered.

### Added

- **`variant_select_mode`** parameter on `pyrer.solve()` —
  `"version_priority"` (rez's default) and `"intersection_priority"`. Mirrors
  rez's `config.variant_select_mode`. On the Rust side: new
  `VariantSelectMode` enum, `SolverContext::with_variant_select_mode(mode)`
  builder, `Solver::new_with_options(reqs, repo, cache, mode)` constructor.
  Closes #63.

### Changed

- **Differential test now enforces variant-index parity.** The 188-case rez
  benchmark gate previously checked `(name, version)` only; it now also
  compares the variant index rez picked for each entry. 188/188 still pass.

## [0.1.0-rc.6] — 2026-05-15

The performance and ergonomics push. Released as the basis for the local
benchmark numbers in the README (~36× rez 3.3.0 on the same machine, Python
3.9; ~19× on Python 3.13).

### Added

- **`PackageData`** input class — `pyrer.solve(requests, packages: list[PackageData])`
  replaces the prior JSON-string repo form.
- **`PackageData.from_rez(pkg)`** — duck-typed convenience for converting a
  rez `Package` (or anything with `name` / `version` / `requires` /
  `variants`) into a `PackageData`. No rez import in `pyrer` itself; the
  helper works in tests via plain Python classes too.
- **`ResolvedVariant`** output class — `result.resolved_packages` returns a
  list of these with `name`, `version`, `variant_index`, `requires`
  (merged base + variant-specific), and a rez-shaped `uri`
  (`"name/version/package.py[idx]"`). `result.resolved` keeps the tuple
  form for callers that prefer it.
- **`SharedVariantCache`** + `make_shared_cache()` + `Solver::new_with_cache`
  — share the family/version variant cache across many solves of the same
  repository. Drops 20 % off the 188-case benchmark on its own.
- **Solver micro-benchmarks** (`crates/rer-resolver/benches/solver_micro.rs`)
  exercising `PackageVariantSlice::{intersect, reduce_by, extract}`,
  `Requirement::conflicts_with`, `VersionRange::{union, intersection,
  intersects}`, and end-to-end small solves. Designed for criterion
  `--save-baseline` / `--baseline` regression detection.
- **Wiring `pyrer` into `rez` documentation**: a new doc page covering the
  integration model, `build_pyrer_packages`, monkey-patch shim, caveats, and
  a sanity-check loop for diffing pyrer vs rez on a real repo.
- **"Why rer doesn't cache `intersect` / `reduce_by` results"** engineering
  note documenting the dedup-cache experiment that was tried, measured at
  2× regression, and abandoned (#64 closed as wontfix).
- **`rer-python` "from_rez" + `ResolvedVariant`** tests
  (`tests/test_rich_api.py`).

### Changed

- **Solver hot-path interning.** Package family names migrated from `String`
  to `Rc<str>` throughout the solver (`requirement.rs`, `scope.rs`,
  `variant.rs`, `phase.rs`). Cuts `String::clone` and HashMap-of-String
  cost. Closes one of two solver-audit findings from the post-#65 analysis.
- **`VersionRange` clones are now refcount bumps.** The inner
  `Ranges<RerVersion>` is wrapped in `Rc` — `clone` is a refcount bump
  instead of a `SmallVec` allocation. Most-cloned type on the hot path.
- **Pre-filter reduction pairs by `fam_requires`.** Phase loop now skips
  `(scope_x, scope_y)` pairs at generation time when `scope_x` has no
  requirement on `scope_y`'s family — drops the per-call function-entry
  cost plus rehash/sort costs from a 27M-entry `pending_set` down to ~400k.
- **`PackageVariantSlice::extractable()` is now O(1)**: rewritten from a
  `HashSet::is_subset` walk to a length compare. Largest single perf win,
  ~30 % on the 188-case benchmark.
- **Per-scope `non_extractable` flag** in the phase loop skips `extract()`
  calls on scopes already known to be exhausted, between intersect/reduce
  passes.
- **`mimalloc` is the global allocator** in the `rez_benchmark_dataset`
  example binary. Dropped wheel-side libc malloc share from ~33 % to ~4 %.
- `pyrer.solve` accepts the `packages` argument as either a
  `list[PackageData]` (the new form) or — in earlier rc.6 commits — a JSON
  string. **By the final rc.6 release the JSON-string form is gone** (see
  *Removed* below); only `list[PackageData]` is accepted.
- **README**: full benchmark section with hardware/version/method, rez 3.3.0
  run locally on the same machine, headline comparison against CPython
  3.13 plus a CPython 3.9 row for context.

### Removed

- **JSON-string `packages` argument** to `pyrer.solve`. rez does not
  produce or consume that shape anywhere; it was a pyrer-internal fixture
  convenience. Callers passing a JSON string must convert to
  `list[PackageData]`; the migration is a one-liner (see PR description in
  the merged commit). `serde_json` dropped from `rer-python`'s `Cargo.toml`
  too.
- **`crates/rer-version/benches/main.rs`** (the legacy `Requirement`
  parser bench) — measured a code path the solver doesn't use. Replaced by
  `crates/rer-resolver/benches/solver_micro.rs`.
- **Six unused workspace dependencies** dropped: `pubgrub`, `rayon`,
  `uuid`, `itertools`, `log`, `log4rs`. Nothing in any crate's manifest
  pulled them after the pubgrub-solver and placeholder-CLI removals.
- **The placeholder `rer` CLI crate** (a `Hello, world!` stub) was
  removed; the `rer` name on crates.io is therefore not claimed by this
  workspace. Real CLI work is future scope.

### Fixed

- **PyPI publish failed at `License-File` validation** on the rc.2 → rc.3
  push (status 400: `License-File LICENSE does not exist in distribution
  file at pyrer-0.1.0rc2/LICENSE`). `rer-python` now sets
  `license = "MIT"` (SPDX) instead of inheriting `license-file = "LICENSE"`
  from the workspace; maturin no longer emits a stray `License-File:`
  line in `PKG-INFO`.

## [0.1.0-rc.5] — 2026-05-15

### Fixed

- **Zola 0.22 schema renames** for the documentation site:
  `generate_feed` → `generate_feeds`, `[markdown] highlight_code = true`
  → `[markdown.highlighting] theme = "catppuccin-mocha"`. Section
  `_index.md` files lost their `date` / `updated` fields (page-only in
  current Zola).

## [0.1.0-rc.4] — 2026-05-15

### Added

- **User-facing documentation rewrite** under `docs/`: replaced the
  AdiDoks theme placeholder content with an actual rer site. New pages:
  introduction (status, faithfulness, what rer is and isn't), quick-start
  (Python + Rust), contributing, FAQ. Authors section aligned with the
  workspace `Cargo.toml` authors list.
- **`config.toml`** rewritten for the real project (title, description,
  repo links, version pill).

### Removed

- The Finnish AdiDoks marketing homepage (`docs/content/_index.fi.md`).

## [0.1.0-rc.3] — 2026-05-15

### Fixed

- The `License-File` problem from `rc.2`'s PyPI publish (see rc.6's
  *Fixed* note for the underlying details — the fix was carried forward
  on every subsequent rc as well).

## [0.1.0-rc.2] — 2026-05-15

### Changed

- **Renamed the Python distribution to `pyrer`.** The PyPI name `rer`
  was already taken; the wheel ships as `pyrer` and the import is also
  `pyrer` (`pip install pyrer`, `import pyrer`). The Rust workspace and
  the crates.io crates (`rer-version`, `rer-resolver`) keep their names.

## [0.1.0-rc.1] — 2026-05-14

Initial release. Establishes the project: a rez-faithful port of rez's
phase-based package solver in Rust, callable from Python via PyO3.

### Added

- **`rer-version` crate** — `RerVersion` (rez token ordering, including
  alphanumeric subtokens), `VersionRange` (rez range semantics over
  [`version-ranges`](https://crates.io/crates/version-ranges)).
- **`rer-resolver` crate** — the `rez_solver` module, a faithful port of
  `rez/src/rez/solver.py`:
  - `Solver` — phase-stack driver with implicit backtracking.
  - `ResolvePhase` — the extract / merge / intersect / reduce / split cycle.
  - `PackageScope` — three kinds (normal, conflict `!`/`~`, ephemeral `.`).
  - `PackageVariantSlice` / `PackageVariantCache` / `PackageVariantList`.
  - `Requirement` / `RequirementList` with weak (`~`) / conflict (`!`)
    semantics.
  - `DepGraph` — cycle detection + dependency ordering for `finalise`.
- **`rer-python` crate (`pyrer` on PyPI)** — PyO3 bridge exposing
  `pyrer.solve(requests, packages_json) -> SolveResult`. (Form has since
  changed; see rc.6 for the current shape.)
- **`rez_benchmark_dataset` example** running rez's bundled 188-case
  benchmark through rer's solver and reporting timings.
- **`test_rez_benchmark` integration test** validating that rer produces
  the same resolved packages as rez records for every benchmark case
  (188 / 188).
- **`scripts/prepare_benchmark_data.py`** — extracts rez's benchmark
  fixtures from the `rez` git submodule into the JSON shape rer's tests
  consume. Pure-AST `package.py` parser, no rez install or `exec()`
  needed.
- **Documentation site** scaffold under `docs/` (Zola, AdiDoks theme).

[Unreleased]: https://github.com/doubleailes/rer/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/doubleailes/rer/compare/v0.1.0-rc.6...v1.0.0
[0.1.0-rc.6]: https://github.com/doubleailes/rer/compare/v0.1.0-rc.5...v0.1.0-rc.6
[0.1.0-rc.5]: https://github.com/doubleailes/rer/compare/v0.1.0-rc.4...v0.1.0-rc.5
[0.1.0-rc.4]: https://github.com/doubleailes/rer/compare/v0.1.0-rc.3...v0.1.0-rc.4
[0.1.0-rc.3]: https://github.com/doubleailes/rer/compare/v0.1.0-rc.2...v0.1.0-rc.3
[0.1.0-rc.2]: https://github.com/doubleailes/rer/compare/v0.1.0-rc.1...v0.1.0-rc.2
[0.1.0-rc.1]: https://github.com/doubleailes/rer/releases/tag/v0.1.0-rc.1
