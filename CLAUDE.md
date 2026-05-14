# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`rer` ("Rez En Rust") is a Rust reimplementation of the solver hotpath of [rez](https://github.com/AcademySoftwareFoundation/rez), the VFX package manager. The end goal is a **hybrid integration**: a Rust library callable from Python via PyO3 that accelerates rez resolves while leaving the rest of rez untouched.

The solver is built on [pubgrub](https://github.com/pubgrub-rs/pubgrub) 0.3 — it is *not* a port of rez's custom backtracking solver. Output must be rez-*compatible*, but the algorithm is intentionally different, so pubgrub may return a different (equally valid) solution for the same input. Correctness is validated, not solution identity.

A much more detailed design/phase document lives in `.github/copilot-instructions.md` — read it for type-level detail, the phased roadmap, and rez reference pointers. Note it has drifted in places: `rer-python`, `package_id.rs`, and `package_filter.rs` already exist despite being listed as "planned".

## Commands

```bash
cargo build                              # build all crates
cargo test                               # all tests
cargo test -p rer-version                # one crate
cargo test -p rer-version test_bump      # one test (name filter)
cargo test -p rer-resolver --test test_differential   # one integration-test file
cargo bench                              # benchmarks (rer-version only)
cargo run -p examples --example new_solver            # examples are a separate crate
```

Python / PyO3 module (`rer_solver`):

```bash
python -m venv .venv && . .venv/bin/activate
pip install maturin pytest
cd crates/rer-python && maturin develop   # builds + installs rer_solver into the venv
pytest tests/test_differential.py -v      # run from repo root
```

Differential testing (Rust solver vs. rez) runs on both sides: `cargo test -p rer-resolver --test test_differential` and `pytest tests/`, sharing `tests/differential_cases.json`.

## Workspace layout

Virtual workspace (root `Cargo.toml` has no `[package]`) — always use `-p <crate>` for crate-specific commands.

- **`rer-version`** — version & requirement types. `RerVersion` (rez token ordering: `_` < lowercase < uppercase < digits), `Requirement`/`Requirements` (rez syntax: `foo-1.0+<2.0`, `~foo` weak, `!foo` conflict, `a|b` union), regex-based parser.
- **`rer-resolver`** — the solver. `RerDependencyProvider` implements pubgrub 0.3's `DependencyProvider` (associated types, no generics). `solver()` is an alternative path that pre-builds an `OfflineDependencyProvider`. `LocalPackages` scans the filesystem for package families. `package_id.rs` / `package_filter.rs` support the in-progress multi-variant and filtering work.
- **`rer-python`** — PyO3 bridge. Crate lib name is `rer_solver`; exposes `solve(requests, package_paths, ...)` returning a `SolveResult`. Built into wheels by maturin (`crates/rer-python/pyproject.toml`, no root `pyproject.toml`).
- **`rer`** — `clap` CLI binary, currently a placeholder.
- **`examples`** — benchmark/usage binaries; a workspace member, not a library.

## Gotchas

- **`Ranges<RerVersion>`** comes from the `version-ranges` crate (`version_ranges::Ranges`), not pubgrub — pubgrub 0.3 extracted it.
- **`get_dependencies()` returns empty `Requirements`.** The legacy `package.py` parser was removed; both `LocalPackages::get_dependencies()` and `RerDependencyProvider::fetch_dependencies()` are stubs. Any test relying on transitive deps needs this resolved first (Phase 0 — the current gap).
- **`Requirements` is a consuming iterator** — iterating it pops/drains the list. Clone before iterating if you need it again.
- **`RerVersion::bump()` appends `_`** (lowest-sorting char), so `Ranges::between(v, v.bump())` matches `v` and anything extending it (`1.0.0` matches `1.0.0.beta`). Verify each range test against rez behavior.
- **`"init"` sentinel** — the solver uses a virtual root package `"init"` at version `1.0.0` for the initial request (to be replaced by `PackageId::Root`).
- The `rez/` directory is a git submodule (upstream rez) used as the behavioral reference.

## Conventions

- Edition 2021, stable toolchain. Run `cargo fmt` before committing; CI runs `cargo build`, `cargo test`, `cargo bench`.
- Add dependency versions to the root `Cargo.toml` `[workspace.dependencies]`, not inline in crate manifests.
- Crate names use hyphens (`rer-version`), lib names use underscores (`rer_version`) via `[lib] name`.
- Conventional commits; scope is the crate name without the `rer-` prefix: `version`, `resolver`, `python`. E.g. `feat(resolver): ...`, `fix(version): ...`, `ci: ...`.
