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

- **Package-orderer plugin SDK.** `pyrer` now exposes
  `pyrer.PackageOrderer` (an SDK base class) and
  `pyrer.register_orderer()` (an explicit registry), mirroring rez's
  own orderer model. A studio subclasses `PackageOrderer`, implements
  `order(family, versions) -> list[str]` (versions reordered
  most-preferred-first), registers it, and selects it via
  `pyrer.solve(..., package_orderer="<name>")` — by registered name or
  by passing an instance. This lets a host override `rer`'s default
  highest-version preference to match a custom rez orderer (e.g. a
  PEP 440 orderer), the root cause of the version-selection divergence
  in #96. The orderer is a preference function — it never changes
  whether a solve succeeds; a misbehaving orderer (omitted/extra
  versions) is handled defensively; a raising `order()` surfaces as
  `status="error"`. On the Rust side: new `FamilyOrderer` callback
  type, `SolverContext::with_package_order` builder,
  `SolverContext.package_order` field, and a 5th `package_order`
  parameter on `Solver::new_with_options`.

### Changed

- **`pyrer` is now a mixed Rust+Python package.** The compiled PyO3
  extension moved to the `pyrer._native` submodule; a pure-Python
  `pyrer` package wraps it and hosts the plugin SDK. `import pyrer`
  and every existing symbol (`solve`, `PackageData`, `SolveResult`,
  `parse_static_package_py`, …) are unchanged for callers — the
  restructure is transparent. `pyrer.__version__` is now available.

## [1.0.0-rc.3] — 2026-05-19

First release candidate of the 1.0 line. Closes the integration
loop with rez: pyrer now ships a Rust `package.py` parser, a
batched parallel I/O path, and a lazy-discovery hook with
constraint-aware filtering, on top of the rez-faithful solver.
The strict 188-case rez differential still passes 188/188 —
public API is from this release forward under the
[Stability commitments](https://doubleailes.github.io/rer/docs/engineering/stability/).

### Added

#### Rust `package.py` parser

- **New crate `rer-package`** — hand-rolled lexer that extracts the
  four solver-relevant fields (`name`, `version`, `requires`,
  `variants`) from a rez `package.py` source string without
  invoking Python. Zero non-stdlib dependencies. Accepts the static
  subset: literal assignments, ignorable `def commands(...)` /
  `def pre_commands(...)`, ignorable `with scope("config")`
  declarative DSL. Bails to `None` (caller falls back to rez) on
  `@early` / `@late`, top-level `if` / `for` / `class`, `import`,
  non-literal assignment to a solver field, or anything the scanner
  doesn't explicitly recognise — biased hard toward bailing so
  silent correctness regressions can't slip in. See the
  [engineering RFC](https://doubleailes.github.io/rer/docs/engineering/fast-package-py-parser/).
- **`pyrer.parse_static_package_py(source) -> Optional[PackageData]`** —
  PyO3 binding for the per-file parser. Returns the four-field
  `PackageData` on success, `None` on any reason to bail. No
  exception ever escapes; even a syntax error becomes `None` (the
  caller would invoke rez on it anyway).
- **`pyrer.parse_static_packages_py(paths) -> list[PackageData | None]`** —
  batched, Rayon-parallel variant. Opens and parses every path in
  one Rust call across a thread pool with the GIL released
  (`Python::allow_threads`). Output is positionally aligned with
  the input; missing files, unreadable bytes, and parser-bails all
  map to `None` at the matching index. Pool size follows
  `RAYON_NUM_THREADS` (default: logical core count). Targets the
  serial-Python-`open()`-loop bottleneck after the per-file parser
  landed (#94 cProfile showed 3.20 s of 9.12 s — 35% of resolve
  wall time — in serial `open()`). Closes #94.

#### `pyrer.solve()` integration surface

- **`load_family` callback** for lazy package discovery — pass
  `load_family: Callable[[str], list[PackageData]]` and pyrer
  invokes it on demand the first time the solver needs a family
  it hasn't seen. Each family is loaded at most once per solve.
  Returning `[]` means "no such family". Aimed at cold-cache and
  network-filesystem integrations (Windows + CIFS in particular)
  where the up-front BFS of every reachable family dominates
  `rez env` wall time. Closes #86.
- **`version_range` hint on `load_family`** — the callback signature
  can now accept a second `version_range` argument (named parameter
  or `**kwargs`); pyrer detects via `inspect.signature` once per
  `solve()`. The hint is a rez-syntax range string (e.g. `"2+<3"`,
  `None` for unconstrained) the shim can pass directly to
  `rez.packages.iter_packages(range_=...)` to skip on-disk version
  directories outside the request. Targets the 95% load-fan-out
  waste documented in #92 (2,637 packages loaded for 132 used on a
  typical Fortiche resolve). The repo tracks the loaded range per
  family and reloads with a widened range if the solver backtracks
  and needs more — see `PackageRepo` notes in **Changed** below.
  Backward-compatible: 1-arg callbacks keep working. Closes #92.
- **`PackageData.from_strings(name, version, requires=None, variants=None)`** —
  classmethod constructor for raw-string callers, symmetric with
  `from_rez(pkg)`. Skips rez's `AttributeForwardMeta` chain, the
  `Requirement` parse, and the `str(Requirement)` round-trip on
  the rez-shim hot path. Functionally equivalent to the four-arg
  constructor; the classmethod form exists so callers wiring
  `pkg.resource.data` through pyrer have a named, documented
  contract to reach for. Falls back to `from_rez` for `@early` /
  `@late`-bound attributes. Closes #88.
- **`resolved_ephemerals`** on `pyrer.SolveResult` — list of
  rez-style ephemeral requirement strings (`[".feature-1.5",
  ".mode-debug"]`) surfaced from the solver, matching
  `rez.solver.Solver.resolved_ephemerals`. Closes #84.
- **`variant_select_mode`** parameter on `pyrer.solve()` —
  `"version_priority"` (rez's default) and `"intersection_priority"`.
  Mirrors `config.variant_select_mode`. New `VariantSelectMode` enum
  on the Rust side, plus `SolverContext::with_variant_select_mode`
  / `Solver::new_with_options` constructors. Closes #63.

#### Rust API additions

- **Borrowing-iterator forms** on `Solver` and `ResolvePhase`:
  `resolved_packages_iter`, `resolved_ephemerals_iter`,
  `iter_solved_variants`, `iter_solved_ephemerals`. Avoid the
  intermediate `Vec` (and, for ephemerals, the per-element
  `Requirement::clone`) when callers just want to iterate.

### Changed

- **`PackageRepo` is now a struct**, not a `HashMap` type alias.
  Carries a cache (`RefCell<HashMap<…>>`), optional `FamilyLoader`
  closure, and a per-family `loaded_range` so `get_family(name,
  hint)` can reload with a widened range when the solver
  backtracks. Construct with `PackageRepo::from_map(map)` for the
  eager case or `PackageRepo::with_loader(loader)` for lazy;
  `From<HashMap<…>>` is implemented for back-compat. Eager-path
  perf unchanged in measurement (within run-to-run noise of the
  README baseline).
- **`FamilyLoader` type signature** is now
  `Fn(&str, Option<&VersionRange>) -> Vec<(String, PackageData)>`
  (was 1-arg) — required to thread the `version_range` hint through.
- **Differential test now enforces variant-index parity.** The
  188-case rez benchmark gate previously checked `(name, version)`
  only; it now also compares the variant index rez picked for each
  entry. 188/188 still pass.

### Performance

Measured on the Fortiche corpus (`/thierry/rez/pkg`, ~6,400 `package.py`
files, served over CIFS) against rez 3.3.0:

- **Static parser, end-to-end**: 75 μs/file (`open + read + parse`)
  vs rez's `DeveloperPackage.from_path + from_rez` at 2,615 μs/file.
  **34.8× speedup.** The parse step alone dropped from 1,990 μs
  (V1 rustpython-parser) to **59 μs (V2 hand-rolled lexer)** —
  the hand-rolled rewrite was a 33× win on its own layer.
- **Corpus accept rate**: 92.9% of files (5,979 / 6,439) are
  statically parseable. Per the issue #84 differential harness,
  the parser produces **0 mismatches** against rez on the
  V2-accepted set.
- **Batched parser**: 2.81× speedup on 2,000-file batches over the
  serial Python `open()` loop the shim ran before (4,234 ms → 1,508
  ms). Per-file saving ~1.36 ms; extrapolated to a 2,600-file
  resolve, ~3.5 s saved per `_load_family`.
- **Solver**: unchanged. Strict 188-case rez differential remains
  188/188.

### Tooling

- **`scripts/survey_package_py.py`** — Stage 1 corpus classifier that
  walks a rez repo and reports per-file: fast-parseable / dynamic-
  requires / imports / top-level-classdef / etc. Pure stdlib; the
  go/no-go signal for whether the parser is worth wiring into a
  given studio.
- **`scripts/diff_against_rez.py`** — Stage 2 safety net. For every
  file `parse_static_package_py` accepts, also load via rez's
  `DeveloperPackage.from_path` and assert the four solver fields
  match byte-for-byte. Run on the full Fortiche corpus: **0
  mismatches on 5,979 V2-accepted files** in 74 seconds. Treats
  any divergence as a release blocker.
- **`scripts/bench_package_py_parser.py`** — full-load comparison
  (`open + read + parse_static_package_py` vs `DeveloperPackage.from_path
  + from_rez`), the source of the 34.8× number.
- **`scripts/bench_batched_parser.py`** — serial-loop vs batched
  comparison, the source of the 2.81× number.
- **`scripts/bench_python_construction.py`** — `PackageData`
  construction microbench (`from_strings` vs `from_rez` paths).
- **`scripts/compare_resolves.py`** — pyrer-vs-rez bisect tool for
  divergence reports. Uses the recommended shim shape internally
  (static parser + `load_family` + `version_range` + `from_rez`
  fallback) and reports per-request agreement / divergence /
  failure. Used to triage #96.

### Docs

- **[Wiring `pyrer` into `rez`](https://doubleailes.github.io/rer/docs/getting-started/rez-integration/)**:
  full production integration guide covering the static parser
  (per-file + batched), `load_family`, `version_range` hint,
  `from_strings`, shadow-validation mode, metrics counters,
  rollout plan, and a "Where this WON'T help" honest-caveat list
  for each feature.
- **[Static `package.py` parser RFC](https://doubleailes.github.io/rer/docs/engineering/fast-package-py-parser/)**:
  Stages 1-4 design + measured results, the V1 → V2 spike story
  (1.7× → 34.8×), the differential safety-net result, and a
  "Considered alternatives" section flagging the parsed-package
  cache as the next architectural lever.
- **FAQ entry** for "Where does rer get package data from?"
  updated to mention both eager and lazy (`load_family`) supply
  paths.

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
