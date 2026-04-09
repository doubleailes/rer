# Copilot Instructions — rer (Rez En Rust)

## Project Overview

`rer` is a Rust reimplementation of the solver and version-resolution hotpath from [rez](https://github.com/AcademySoftwareFoundation/rez), the VFX/animation industry package manager. The goal is a **hybrid integration**: a Rust library callable from Python via PyO3 that accelerates rez resolves while keeping the rest of rez untouched.

The solver uses [pubgrub](https://github.com/pubgrub-rs/pubgrub) (version-solving algorithm) as its backend, not a port of rez's custom solver. Results must be compatible with rez but the algorithm is intentionally different.

## Repository Structure

```
rer/
├── Cargo.toml                    # Workspace root
├── crates/
│   ├── rer-version/              # Version types and requirement parsing
│   │   ├── src/
│   │   │   ├── version.rs        # RerVersion, AlphanumericVersionToken, SubToken
│   │   │   ├── requirement/
│   │   │   │   ├── requirement.rs  # Requirement, Requirements
│   │   │   │   └── parser.rs      # Version range parser (regex-based)
│   │   │   └── lib.rs
│   │   ├── tests/
│   │   └── benches/
│   ├── rer-resolver/             # Solver and dependency resolution
│   │   └── src/
│   │       ├── resolver.rs       # RerDependencyProvider (pubgrub DependencyProvider impl)
│   │       ├── solver.rs         # Standalone solver function using OfflineDependencyProvider
│   │       ├── candidate_selector.rs  # CandidateList, ResolutionMode
│   │       └── local_package.rs  # Filesystem package discovery and caching
│   ├── test-rust-python/         # package.py AST parser using rustpython-parser
│   │   └── src/lib.rs            # Package struct — reads static assignments from package.py
│   ├── examples/                 # Benchmark and usage examples
│   └── rer/                      # CLI binary (minimal, uses clap)
├── data_set/                     # Test data (JSON package repos)
├── python/                       # Python helper scripts
└── docs/                         # Zola-based documentation site
```

## Key Types and Their Roles

### `rer-version` crate

- **`RerVersion`** — Rez-compatible version. Tokens separated by `.`, `-`, etc. Each token is an `AlphanumericVersionToken` containing `SubToken`s. Implements pubgrub's `Version` trait (`lowest()`, `bump()`). Ordering follows rez semantics: `_` < lowercase < uppercase < digits.
- **`Requirement`** — A package name + optional `Range<RerVersion>`. Supports rez syntax: `foo-1.2`, `foo-1.0+<2.0`, `~foo-1` (weak ref), `!foo-1` (conflict), `foo-1.2|2.1` (union).
- **`Requirements`** — A list of `Requirement` with `merge()` (intersect same-name ranges), `split_conflict()`, `split_weak_ref()`, and conversion to pubgrub format via `to_pubgrub()`.

### `rer-resolver` crate

- **`RerDependencyProvider`** — Implements pubgrub's `DependencyProvider` trait. Discovers packages from filesystem paths, parses `package.py` for dependencies, and feeds them to pubgrub's `resolve()`.
- **`CandidateList`** — Sorted list of versions for a package family. `find_candidate()` picks the best version matching a range and `ResolutionMode` (currently `Highest` or `Lowest`).
- **`LocalPackages`** — Lazy filesystem scanner. Caches version→path mappings per package family.
- **`solver()`** function — Alternative solver path that pre-builds an `OfflineDependencyProvider` recursively, then calls pubgrub `resolve()`.

### `test-rust-python` crate

- **`Package`** — Parses `package.py` files using `rustpython-parser` (AST only, no eval). Extracts `name`, `version`, `requires`, `variants`, `build_requires`, `authors`, and other static attributes. **Cannot handle dynamic Python** (conditionals, `late()`, `this`, function-based requires).

## Coding Conventions

### Rust

- Edition 2021. Stable toolchain.
- Use workspace dependencies defined in the root `Cargo.toml` — do not add versions inline in crate `Cargo.toml` files.
- Run `cargo fmt` before committing. Follow standard `clippy` recommendations — the CI runs `cargo test` and `cargo bench`.
- Prefer `thiserror` for error types over manual `impl Display + Error`. When adding new error types, make them `enum` variants, not stringly-typed.
- Use `#[cfg(test)]` inline tests for unit tests. Use `crates/*/tests/` for integration tests.
- All public types and functions must have doc comments (`///`). Include a brief description and at least one example for parsers and constructors.
- When implementing traits from pubgrub (`Version`, `DependencyProvider`), keep the implementations focused — do not add unrelated logic inside trait methods.

### Naming

- Crate names use hyphens: `rer-version`, `rer-resolver`.
- Lib names use underscores: `rer_version`, `rer_resolver` (set in `Cargo.toml [lib] name`).
- Types use `Rer` prefix only for the top-level version type (`RerVersion`). Internal types do not need it (`Requirement`, `CandidateList`, `PackageFilter`).
- Test functions: `test_<what>_<scenario>` (e.g., `test_merge_requirement_conflict`).

### Testing

- Every new public function needs at least one test.
- Version comparison tests must cover: numeric vs alpha tokens, `_` ordering, separator handling, `bump()` correctness.
- Solver tests must use small, self-contained package repos — either inline JSON or fixture files in `data_set/`. Do not depend on hardcoded absolute paths.
- When a test compares rer output against expected rez behavior, add a comment linking to the relevant rez test or documentation.

## Current Development Priorities

The project is executing a phased plan to integrate as a hybrid solver for rez. When working on issues, follow these priorities:

### Phase 1 — Multi-Variant Correctness (CURRENT FOCUS)

The most critical gap. Multi-variant packages must be encoded as indexed sub-packages in pubgrub:
- `foo[0]/1.0.0` → `requires` + variant 0 deps
- `foo[1]/1.0.0` → `requires` + variant 1 deps
- Virtual `foo/1.0.0` → picks exactly one variant

This requires introducing a `PackageId` enum (`Base`, `Variant(name, index)`, `Root`) to replace the raw `String` package identifier used throughout.

### Phase 2 — Package Filter & Ordering

Add a `PackageFilter` trait (with `RegexFilter`, `GlobFilter`, `TimestampFilter`) and extend `ResolutionMode` to support per-family ordering and `VersionSplit`.

### Phase 3 — PyO3 Bridge

New `rer-python` crate exposing a `solve()` function callable from Python. Built with maturin. Returns a `SolveResult` struct.

### Phase 4+ — Integration, Testing, Performance

Differential testing against rez, dependency parse caching, CI wheel builds.

## Architecture Decisions

### Why pubgrub instead of porting rez's solver

Rez's solver is a custom backtracking algorithm (~2,400 lines). Pubgrub is a well-tested, community-maintained algorithm with better worst-case behavior. The trade-off is that pubgrub may produce different (but equally valid) solutions for the same input. This is acceptable — we validate correctness, not solution identity.

### Why `rustpython-parser` for `package.py`

Rez `package.py` files are executable Python. Embedding a full Python interpreter (PyO3) would negate the performance benefit. Instead, we parse the AST and extract static assignments. Packages with dynamic `requires` (conditionals, `late()`, function calls) should be detected and rejected — the Python side falls back to rez's native solver for these.

### Data flow in hybrid mode

```
Python (rez)                          Rust (rer)
─────────────                         ──────────
                                      
Resolver._solve_rust()                
  → serialize requests + paths        
  → call rer_solver.solve()  ────►   RerDependencyProvider
                                       → reads filesystem
                                       → parses package.py (static only)
                                       → pubgrub resolve()
  ← SolveResult              ◄────   return (status, resolved, timing)
  → map back to rez objects           
```

Rust owns filesystem I/O and package parsing. Python passes paths and requests as strings, receives results as `(name, version, variant_index)` tuples.

## Things to Watch Out For

1. **pubgrub `Range<RerVersion>` is not rez `VersionRange`** — pubgrub ranges don't have a human-readable string representation that matches rez syntax. When displaying ranges to users, convert back through `Requirement::to_string()`.

2. **`RerVersion::bump()` semantics** — pubgrub uses `bump()` to create exclusive upper bounds. Our implementation appends `_` (the lowest-sorting character). This means `Range::between(v, v.bump())` matches `v` and any version that extends it (e.g., `1.0.0` matches `1.0.0.beta`). Verify this matches rez behavior for each new range test.

3. **The `"init"` / `Root` sentinel** — The solver uses a virtual root package to represent the initial request. Currently this is the string `"init"` with a fixed version `1.0.0`. Phase 1 replaces this with `PackageId::Root`.

4. **`Requirements` is a consuming iterator** — `Requirements` implements `Iterator` by calling `self.0.pop()`. This means iterating consumes the list. Clone before iterating if you need the data again. Consider refactoring to return `impl Iterator` via `.iter()` instead.

5. **Hardcoded paths in tests** — Some tests in `local_package.rs` and examples reference absolute paths from a developer machine. New tests must use relative paths, `env!("CARGO_MANIFEST_DIR")`, or inline data. Fix existing ones when you touch those files.

6. **`test-rust-python` crate naming** — This crate parses `package.py` and is core infrastructure, not a test utility. It will likely be renamed in a future cleanup (e.g., `rer-package-parser`).

## Commit Messages

Use conventional commits:

- `feat(resolver): encode multi-variant packages as indexed sub-packages`
- `fix(version): correct SubToken ordering for mixed alpha-numeric`
- `test(solver): add multi-variant resolve test with 3 variants`
- `refactor(resolver): replace String package name with PackageId enum`
- `docs(version): add examples for RerVersion::bump()`
- `ci: add maturin wheel build for rer-python`

Scope is the crate name without the `rer-` prefix: `version`, `resolver`, `package-parser`, `python`.

## Running the Project

```bash
# Build all crates
cargo build

# Run all tests
cargo test

# Run benchmarks
cargo bench

# Run the resolver example (requires package data)
cargo run --example new_solver

# Run the benchmark comparison (requires data_set_private/)
cargo run --example rez_benchmark
```

## Dependencies to Know

| Crate | Role | Docs |
|---|---|---|
| `pubgrub` 0.3.0 | Version solving algorithm | https://github.com/pubgrub-rs/pubgrub |
| `rustpython-parser` 0.3.1 | Python AST parsing | https://github.com/RustPython/Parser |
| `regex` | Version/requirement tokenization | |
| `lazy_static` | Compiled regex caching | |
| `serde` + `serde_json` | Package repo JSON serialization | |
| `rayon` | Parallel candidate selection (experimental) | |
| `uuid` | Root package identity in solver | |
| `pyo3` (future) | Python ↔ Rust bridge | https://pyo3.rs |
| `maturin` (future) | Build Python wheels from Rust | https://maturin.rs |

## Reference: rez Solver Behavior

When verifying solver correctness, the authoritative reference is rez's source:
- Solver: `src/rez/solver.py` — especially `_ResolvePhase`, `_PackageScope`, `Solver.solve()`
- Version: `src/rez/version/_version.py` — `Version`, `VersionRange`, `AlphanumericVersionToken`
- Requirement: `src/rez/version/_requirement.py` — `Requirement`, `RequirementList`
- Tests: `src/rez/tests/test_solver.py`, `src/rez/tests/test_version.py`
- Docs: https://rez.readthedocs.io/en/stable/package_definition.html