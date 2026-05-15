+++
title = "Stability commitments (1.0 onwards)"
description = "What 1.0 commits us to: supported Python versions, supported rez range, semver scope, and what is explicitly out of scope."
draft = false
weight = 20
sort_by = "weight"
template = "docs/page.html"

[extra]
lead = "What 1.0 commits us to — semver scope, supported runtimes, what's modelled, and what is explicitly out of scope."
toc = true
top = false
+++

This page describes the contract that takes effect at **1.0.0** — what
callers can rely on, what is explicitly excluded, and how breaking
changes are handled. For the running list of releases see the
[CHANGELOG](https://github.com/doubleailes/rer/blob/main/CHANGELOG.md).

## Versioning

`rer` follows [Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html).

| Component | Versioned as |
|---|---|
| `rer-version`, `rer-resolver` (Rust crates on crates.io) | One workspace version, bumped together. |
| `pyrer` (PyPI wheel built from `rer-python`) | Same workspace version, published in lock-step. |

From 1.0.0 onwards:

- **MAJOR** — a backwards-incompatible change to the public surface
  listed under [Stable API](#stable-api) below.
- **MINOR** — backwards-compatible additions to the same public surface
  (new methods, new optional parameters with defaults, new resolver
  capabilities).
- **PATCH** — bug fixes and performance improvements that do not change
  observable resolution behaviour on any input.

Pre-1.0.0 (`0.1.0-rc.x`) releases did not carry a stability contract;
the JSON-string `packages` form, the tuple-only `resolved` output, and
several internal type names were reshaped between rcs. See the
CHANGELOG for the path.

## Stable API

These are covered by semver from 1.0.0:

### Python — `pyrer`

- `pyrer.solve(package_requests, packages, *, variant_select_mode=..., …)`:
  signature, return shape, accepted argument types.
- `pyrer.PackageData(name, version, requires=None, variants=None)`:
  constructor, fields, `from_rez(pkg)` classmethod.
- `pyrer.ResolvedVariant`: attribute set (`name`, `version`,
  `variant_index`, `requires`, `uri`).
- `pyrer.SolveResult`: attribute set (`status`, `resolved_packages`,
  `resolved`, `failure_description`, `solve_time_ms`,
  `num_iterations`) and `__repr__` shape.
- Error handling contract: solve failures and bad input come through
  `result.status`; only argument-type errors raise (`TypeError` /
  `ValueError`).

### Rust — `rer-version`, `rer-resolver`

- All types and functions re-exported from `rer_resolver::rez_solver`:
  `Solver`, `ResolvePhase`, `PackageScope`, `PackageVariantSlice`,
  `Requirement`, `RequirementList`, `SolverStatus`, `FailureReason`,
  `VariantSelectMode`, `SharedVariantCache`, `make_shared_cache`,
  `PackageRepo`.
- `Solver::new`, `Solver::new_with_cache`, `Solver::new_with_options`,
  `Solver::solve`, `Solver::status`, `Solver::resolved_packages`,
  `Solver::failure_description`, `Solver::num_solves`,
  `Solver::num_fails`.
- `rer_resolver::PackageData` (the in-memory unit of the package
  repository).
- `rer_version::RerVersion`, `rer_version::VersionRange` and their
  public methods.

### What is *not* covered

Anything not listed above is internal and may change without a major
bump. In particular:

- Module layout below `rer_resolver::rez_solver` (`phase`, `variant`,
  `scope`, `context` submodules and their internal helpers).
- `Reduction`, `Cyclic`, internal failure detail structures —
  available, but the wording of `failure_description` strings is *not*
  covered.
- The `solver_micro` criterion benches — they exist as a regression
  baseline tool, not a public API.
- Internal performance characteristics. We will continue to
  re-optimise; absolute timings are not part of the contract.

## Supported runtimes

### Python

| Stage | Status |
|---|---|
| **3.9** | Supported (the wheel is built `abi3-py39`). |
| **3.10** | Supported. |
| **3.11** | Supported. |
| **3.12** | Supported. |
| **3.13** | Supported. The README's headline benchmark uses 3.13. |
| **3.14+** | Will be supported as soon as `pyo3` ships a compatible release. |
| **3.8 and earlier** | Not supported (Python's own EOL). |

Because the wheel is built against the **stable ABI**
(`abi3-py39`), one wheel per platform / architecture covers every
supported Python — no per-Python wheel matrix.

### Rust

- Edition 2021, stable toolchain.
- MSRV is the current stable toolchain at release time. We do not
  pin an older MSRV; we will note any uplift in the CHANGELOG.

### Operating systems

Tested in CI and supported as production targets:

- **Linux** (manylinux + musllinux), x86_64 / aarch64 / armv7 / s390x /
  ppc64le.
- **macOS**, x86_64 + aarch64.
- **Windows**, x64 / x86 / aarch64.

### `rez`

`pyrer` does **not** import `rez` and works standalone. When using the
optional `PackageData.from_rez(pkg)` integration, the rez range we
explicitly run against is:

| rez version | How it's exercised |
|---|---|
| **3.3.0** | The vendored `rez` git submodule. The local benchmark numbers in the README come from this version. |
| **2.112.0** | The version of rez that produced rez's own published benchmark artifact (Azure VM, 2-core, Nov 2022) — used as upstream context only. |

`pyrer`'s differential test against `rez/src/rez/data/benchmarking/` is
the load-bearing correctness check; both rez versions in that range
ship the same benchmark fixtures.

## Correctness contract

For every resolve, `pyrer` produces:

- The **same `(name, version)` set** as `rez` on the rez 188-case
  benchmark (188 / 188).
- The **same variant index** rez picked for each entry (enforced from
  rc.6's variant-index parity update).
- The **same overall status** — `"solved"` ↔ `success`, `"failed"` ↔
  rez's `failed` or `cyclic`.

A divergence on any of these on any benchmark case is a release
blocker. The differential is run on every CI PR that touches the
resolver. See
[`crates/rer-resolver/tests/test_rez_benchmark.rs`](https://github.com/doubleailes/rer/blob/main/crates/rer-resolver/tests/test_rez_benchmark.rs)
for the gate.

This is `rez`-faithfulness on the benchmark fixtures, **not** a
proof of equivalence on every conceivable input. We treat real-world
divergences as bugs and accept reports against any package set.

## What's modelled

| rez feature | `pyrer` |
|---|---|
| Phase-stack solver, extract / intersect / reduce / split | ✅ Ported faithfully (`rez_solver` mirrors `rez/src/rez/solver.py`). |
| Weak (`~`) and conflict (`!`) requirements | ✅ |
| Ephemeral (`.`) requirements | ✅ |
| `variant_select_mode = version_priority` | ✅ Default. |
| `variant_select_mode = intersection_priority` | ✅ Pass `variant_select_mode="intersection_priority"`. |
| Cycle detection (`finalise` step + dependency order) | ✅ |
| Rez-style `Variant.uri` format on output | ✅ `"name/version/package.py[idx]"`. |
| Variant cache shared across solves | ✅ `SharedVariantCache` + `Solver::new_with_cache`. |

## What's *not* modelled (and how to deal with it)

If your studio depends on any of these, fall back to rez's solver for
the affected resolves. The
[Wiring `pyrer` into `rez`](../../getting-started/rez-integration/)
guide shows the minimal fallback pattern.

| rez feature | `pyrer` |
|---|---|
| `@early` / `@late` evaluated requires | ❌ `pyrer` takes already-parsed requirement strings. Evaluate in rez first, then pass the result through `PackageData.from_rez(pkg)`. |
| Custom `PackageOrder` / `PackageFilter` | ❌ Anything that hooks into rez's order/filter machinery runs in rez. The integration shim falls back when these are non-default. |
| Filesystem-side package discovery (FS walk + `package.py` exec) | ❌ rez does this; `pyrer` consumes the resulting `Package` objects. Tracked as a future feature in issue [#80](https://github.com/doubleailes/rer/issues/80). |
| Resolved-context (`.rxt`) read/write | ❌ `pyrer` doesn't read or write `.rxt`. rez's own `ResolvedContext.load/save` handles those. |
| Build-system / suite / bundle support | ❌ Out of scope. |

## How breaking changes are handled

When a public-API change is required:

1. Open an RFC issue describing the change, the rationale, and the
   migration path.
2. Ship the new shape as an addition with the old one deprecated.
3. Wait at least one MINOR release before removing the old shape.
4. The removal bumps MAJOR.

Exceptions: a security fix that requires a tightening of input
validation may land in a MINOR if the resulting behaviour difference
is itself a fix.

## Performance is not in the contract

The README's benchmark numbers are measurements, not guarantees. We
make a best effort to keep `pyrer` faster than rez on the same
hardware, but the gap will fluctuate as rez evolves and as we tune
further. Specifically:

- Future versions may be slower on a workload if it improves
  correctness, faithfulness, or another workload.
- The `solver_micro` criterion baselines exist to catch regressions
  on the *internal* hot path, not to commit to absolute numbers.
- Cross-version benchmarking numbers are always **same-machine** to
  be honest; see the README's *Reference run* table.

## Reporting a regression

If you find a request that resolves differently in `pyrer` than in
`rez`:

1. Confirm against the latest `pyrer` on PyPI and the rez version
   you're comparing to.
2. Open an issue with the request list, a minimal package set that
   reproduces, both resolutions, and the `pyrer` / rez versions.
3. The project's correctness bar is "match rez 1:1"; we treat any
   divergence as a release blocker.

The 188-case rez benchmark is the authoritative reference — if a
case from that suite ever diverges, that's automatically a release
blocker. See the
[CHANGELOG](https://github.com/doubleailes/rer/blob/main/CHANGELOG.md)
for how prior divergences were handled.
