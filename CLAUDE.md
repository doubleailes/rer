# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`rer` ("Rez En Rust") is a Rust reimplementation of the solver hotpath of [rez](https://github.com/AcademySoftwareFoundation/rez), the VFX package manager. The end goal is a **hybrid integration**: a Rust library callable from Python via PyO3 that accelerates rez resolves while leaving the rest of rez untouched.

The solver (`rer-resolver`'s `rez_solver` module) is a **faithful port of rez's own phase-based backtracking solver** (`rez/src/rez/solver.py`). It reproduces rez's algorithm — weak (`~`) / conflict (`!`) requirement semantics, variant selection order, backtracking — so resolves match rez **1:1**. The earlier pubgrub-based solver was removed once the port matched rez bit-for-bit on rez's bundled 188-case benchmark.

`.github/copilot-instructions.md` has the original phased roadmap and rez reference pointers, but has drifted — trust this file and the code over it.

## Commands

```bash
cargo build                              # build all crates
cargo test                               # all tests
cargo test -p rer-version                # one crate
cargo test -p rer-version test_bump      # one test (name filter)
cargo bench                              # solver micro-benches (rer-resolver/solver_micro)
cargo run --release -p examples --example rez_benchmark_dataset   # timing report
```

The rez benchmark integration test is `#[ignore]`d (the full release run is several minutes):

```bash
git submodule update --init                       # the `rez` submodule
python scripts/prepare_benchmark_data.py          # generates data_set/benchmark_*.json
cargo test --release -p rer-resolver --test test_rez_benchmark -- --ignored
```

Python / PyO3 module (`pyrer` — `rer-python` crate, ships to PyPI as `pyrer`
because `rer` is already taken on PyPI):

```bash
python -m venv .venv && . .venv/bin/activate
pip install maturin pytest
cd crates/rer-python && maturin develop   # builds + installs `pyrer` into the venv
pytest tests/test_differential.py -v      # run from repo root
```

## Workspace layout

Virtual workspace (root `Cargo.toml` has no `[package]`) — always use `-p <crate>` for crate-specific commands.

- **`rer-version`** — version & range types. `RerVersion` (rez token ordering; `Rc`-wrapped internals so clones are cheap), `VersionRange` (a faithful layer over `version_ranges::Ranges`), and a regex-based `Requirement`/`Requirements` parser. Note the solver does **not** use `rer_version::Requirement` — `rez_solver` has its own faithful `Requirement`.
- **`rer-resolver`** — the solver. `rez_solver` is the rez port: `Solver` (phase-stack driver), `ResolvePhase`, `PackageScope`, the variant structures, `Requirement`/`RequirementList`. `PackageData` is the in-memory unit of the package repository (`PackageRepo = HashMap<family, HashMap<version, PackageData>>`); rer never reads the filesystem — the host hands the data in.
- **`rer-python`** — PyO3 bridge. Crate lib name is `pyrer` (so `import pyrer` loads the cdylib) and ships to PyPI as `pyrer` (since `rer` is taken on PyPI); exposes `solve(requests, packages, ...)` where `packages` is the repository as JSON, returning a `SolveResult`. Built into wheels by maturin (`crates/rer-python/pyproject.toml`, no root `pyproject.toml`).
- **`examples`** — `rez_benchmark_dataset`, a timing report; a workspace member, not a library.

## Gotchas

- **The solver works on in-memory `PackageData`** — there is no `package.py` parser in Rust. The benchmark fixtures come from `scripts/prepare_benchmark_data.py`; `rer-python` callers pass the repository as JSON.
- **`rez_solver` has its own `Requirement`** — distinct from the legacy `rer_version::Requirement`. Use `rer_resolver::rez_solver::Requirement` for solver work.
- **`Requirement::parse` panics on a syntactically invalid range** — `rer-python` guards the FFI boundary with `catch_unwind`; in-process callers should pass valid strings.
- **`RerVersion::bump()` appends `_`** (lowest-sorting char), so a `[v, v.bump())` range matches `v` and anything extending it (`1.0.0` matches `1.0.0.beta`). Verify each range test against rez behavior.
- **`test_rez_benchmark` is `#[ignore]`d** and chunkable via `BENCH_RANGE=start:end`; the `benchmark` CI workflow runs it `--release -- --ignored`. Its fixtures are gitignored.
- The `rez/` directory is a git submodule (upstream rez) used as the behavioral reference.

## Conventions

- Edition 2021, stable toolchain. Run `cargo fmt` before committing; CI runs `cargo build`, `cargo test`, `cargo bench`.
- Add dependency versions to the root `Cargo.toml` `[workspace.dependencies]`, not inline in crate manifests.
- Crate names use hyphens (`rer-version`), lib names use underscores (`rer_version`) via `[lib] name`.
- Conventional commits; scope is the crate name without the `rer-` prefix: `version`, `resolver`, `python`. E.g. `feat(resolver): ...`, `fix(version): ...`, `ci: ...`.
- For code edits use the Edit tool or a Python script — not perl one-liners.
