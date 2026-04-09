# Copilot Instructions — rer (Rez En Rust)

## Project Overview

`rer` is a Rust reimplementation of the solver and version-resolution hotpath from [rez](https://github.com/AcademySoftwareFoundation/rez), the VFX/animation industry package manager. The goal is a **hybrid integration**: a Rust library callable from Python via PyO3 that accelerates rez resolves while keeping the rest of rez untouched.

The solver uses [pubgrub](https://github.com/pubgrub-rs/pubgrub) 0.3.0 as its backend, not a port of rez's custom solver. Results must be compatible with rez but the algorithm is intentionally different.

## Repository Structure

```
rer/
├── Cargo.toml                    # Workspace root (virtual — no [package])
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
│   │   ├── src/
│   │   │   ├── resolver.rs       # RerDependencyProvider (pubgrub DependencyProvider impl)
│   │   │   ├── solver.rs         # Standalone solver function using OfflineDependencyProvider
│   │   │   ├── candidate_selector.rs  # CandidateList, ResolutionMode
│   │   │   └── local_package.rs  # Filesystem package discovery and caching
│   │   └── tests/fixtures/       # Test package repos
│   ├── examples/                 # Benchmark and usage examples (separate crate)
│   └── rer/                      # CLI binary (minimal, uses clap)
├── data_set/                     # Test data (JSON package repos)
└── docs/                         # Zola-based documentation site
```

## Key Types and Their Roles

### `rer-version` crate

- **`RerVersion`** — Rez-compatible version. Tokens separated by `.`, `-`, etc. Each token is an `AlphanumericVersionToken` containing `SubToken`s. Ordering follows rez semantics: `_` < lowercase < uppercase < digits. Provides `bump()` and `lowest()` as inherent methods (used for range construction, no longer via the pubgrub `Version` trait).
- **`Requirement`** — A package name + optional `Ranges<RerVersion>`. Supports rez syntax: `foo-1.2`, `foo-1.0+<2.0`, `~foo-1` (weak ref), `!foo-1` (conflict), `foo-1.2|2.1` (union).
- **`Requirements`** — A list of `Requirement` with `merge()` (intersect same-name ranges), `split_conflict()`, `split_weak_ref()`, and conversion to pubgrub format via `to_pubgrub()`.

### `rer-resolver` crate

- **`RerDependencyProvider`** — Implements pubgrub 0.3's `DependencyProvider` trait with associated types (`P = String`, `V = RerVersion`, `VS = Ranges<RerVersion>`, `Priority = Reverse<usize>`). Discovers packages from filesystem paths and feeds them to pubgrub's `resolve()`. Uses `prioritize()` (fewest-matching-versions heuristic) and `choose_version()` (highest matching version via `CandidateList`).
- **`RerSolverError`** — Custom error type implementing `Error + 'static`, used as `DependencyProvider::Err`.
- **`CandidateList`** — Sorted list of versions for a package family. `find_candidate()` picks the best version matching a range and `ResolutionMode` (currently `Highest` or `Lowest`).
- **`LocalPackages`** — Lazy filesystem scanner. Caches version→path mappings per package family. The `get_dependencies()` method currently returns empty `Requirements` — the legacy `package.py` parsing has been removed and a new dependency-reading strategy is needed (see Current Development Priorities below).
- **`solver()`** function — Alternative solver path that pre-builds an `OfflineDependencyProvider` recursively, then calls pubgrub `resolve()`.

### `examples` crate

Benchmark and usage examples. This is a **separate workspace member crate**, not a library — run examples with:
```bash
cargo run -p examples --example new_solver
cargo run -p examples --example rez_benchmark
```

### `rer` crate

Minimal CLI binary using `clap`. Currently a placeholder (`Hello, world!`).

## Coding Conventions

### Rust

- Edition 2021. Stable toolchain.
- Use workspace dependencies defined in the root `Cargo.toml` — do not add versions inline in crate `Cargo.toml` files.
- Run `cargo fmt` before committing. Follow standard `clippy` recommendations — the CI runs `cargo build`, `cargo test`, and `cargo bench`.
- Prefer `thiserror` for new error types over manual `impl Display + Error`. When adding new error types, make them `enum` variants, not stringly-typed. (Note: `RerSolverError` currently uses a manual impl — consider migrating.)
- Use `#[cfg(test)]` inline tests for unit tests. Use `crates/*/tests/` for integration tests.
- All public types and functions must have doc comments (`///`). Include a brief description and at least one example for parsers and constructors.
- When implementing traits from pubgrub (`DependencyProvider`), keep the implementations focused — do not add unrelated logic inside trait methods.

### Naming

- Crate names use hyphens: `rer-version`, `rer-resolver`.
- Lib names use underscores: `rer_version`, `rer_resolver` (set in `Cargo.toml [lib] name`).
- Types use `Rer` prefix only for the top-level version type (`RerVersion`). Internal types do not need it (`Requirement`, `CandidateList`, `PackageFilter`).
- Test functions: `test_<what>_<scenario>` (e.g., `test_merge_requirement_conflict`).

### Testing

- Every new public function needs at least one test.
- Version comparison tests must cover: numeric vs alpha tokens, `_` ordering, separator handling, `bump()` correctness.
- Solver tests must use small, self-contained package repos — either inline JSON or fixture directories under `crates/rer-resolver/tests/fixtures/`. Use `env!("CARGO_MANIFEST_DIR")` for paths, never hardcoded absolute paths.
- When a test compares rer output against expected rez behavior, add a comment linking to the relevant rez test or documentation.

## Current Development Priorities

The project is executing a phased plan to integrate as a hybrid solver for rez. When working on issues, follow these priorities:

### Phase 0 — Dependency Reading (CURRENT GAP)

The `package.py` parsing that previously existed (via `rustpython-parser` in a `test-rust-python` crate) has been removed. Both `LocalPackages::get_dependencies()` and `RerDependencyProvider::fetch_dependencies()` currently return empty `Requirements`. A new strategy for reading package dependencies is needed — options include:

- Re-introducing a `package.py` AST parser (e.g., via `rustpython-parser` or `ruff_python_parser`) in a dedicated crate
- Reading dependencies from a JSON/YAML sidecar format
- Receiving pre-loaded dependency data from Python via the PyO3 bridge

### Phase 1 — Multi-Variant Correctness

Multi-variant packages must be encoded as indexed sub-packages in pubgrub:
- `foo[0]/1.0.0` → `requires` + variant 0 deps
- `foo[1]/1.0.0` → `requires` + variant 1 deps
- Virtual `foo/1.0.0` → picks exactly one variant

This requires introducing a `PackageId` enum (`Base`, `Variant(name, index)`, `Root`) to replace the raw `String` used as `DependencyProvider::P`.

### Phase 2 — Package Filter & Ordering

Add a `PackageFilter` trait (with `RegexFilter`, `GlobFilter`, `TimestampFilter`) and extend `ResolutionMode` to support per-family ordering and `VersionSplit`.

### Phase 3 — PyO3 Bridge

New `rer-python` crate exposing a `solve()` function callable from Python. Built with maturin. Returns a `SolveResult` struct.

### Phase 4+ — Integration, Testing, Performance

Differential testing against rez, dependency parse caching, CI wheel builds.

## Architecture Decisions

### Why pubgrub instead of porting rez's solver

Rez's solver is a custom backtracking algorithm (~2,400 lines). Pubgrub is a well-tested, community-maintained algorithm with better worst-case behavior. The trade-off is that pubgrub may produce different (but equally valid) solutions for the same input. This is acceptable — we validate correctness, not solution identity.

### Data flow (current)

```
Rust (rer)
──────────

RerDependencyProvider
  → reads filesystem for version directories
  → get_dependencies() currently returns empty Requirements (TODO)
  → pubgrub resolve()
  → returns solution as Vec<(package, version)>
```

### Data flow (planned hybrid mode)

```
Python (rez)                          Rust (rer)
─────────────                         ──────────

Resolver._solve_rust()
  → serialize requests + paths
  → call rer_solver.solve()  ────►   RerDependencyProvider
                                       → reads filesystem (or receives pre-loaded data)
                                       → pubgrub resolve()
  ← SolveResult              ◄────   return (status, resolved, timing)
  → map back to rez objects
```

## Things to Watch Out For

1. **`Ranges<RerVersion>` is from `version-ranges` crate, not pubgrub** — pubgrub 0.3 moved `Range<V>` to a separate `version-ranges` crate as `Ranges<V>`. Import from `version_ranges::Ranges`, not pubgrub.

2. **`RerVersion::bump()` semantics** — `bump()` appends `_` (the lowest-sorting character). This means `Ranges::between(v, v.bump())` matches `v` and any version that extends it (e.g., `1.0.0` matches `1.0.0.beta`). Verify this matches rez behavior for each new range test.

3. **The `"init"` sentinel** — The solver uses a virtual root package named `"init"` with a fixed version `1.0.0` to represent the initial request. Phase 1 will replace this with `PackageId::Root`.

4. **`Requirements` is a consuming iterator** — `Requirements` implements `Iterator` by calling `self.0.pop()`. This means iterating consumes the list. Clone before iterating if you need the data again. Consider refactoring to return `impl Iterator` via `.iter()` instead.

5. **`get_dependencies()` returns empty results** — Both `LocalPackages::get_dependencies()` and `RerDependencyProvider::fetch_dependencies()` currently return empty `Requirements` because the legacy `package.py` parser was removed. Any solver test that depends on transitive dependencies will need this fixed first.

6. **Virtual workspace root** — The repo root is a virtual workspace (`Cargo.toml` has no `[package]`). Use `-p <crate>` when running specific crates. Examples: `cargo test -p rer-version`, `cargo run -p examples --example new_solver`.

7. **pubgrub 0.3 `DependencyProvider` uses associated types** — The trait no longer takes generic parameters. Package type, version type, version set, priority, and error are all associated types on the impl. See `resolver.rs` for the current implementation.

## Commit Messages

Use conventional commits:

- `feat(resolver): encode multi-variant packages as indexed sub-packages`
- `fix(version): correct SubToken ordering for mixed alpha-numeric`
- `test(solver): add multi-variant resolve test with 3 variants`
- `refactor(resolver): replace String package name with PackageId enum`
- `docs(version): add examples for RerVersion::bump()`
- `ci: add maturin wheel build for rer-python`

Scope is the crate name without the `rer-` prefix: `version`, `resolver`, `python`.

## Running the Project

```bash
# Build all crates
cargo build

# Run all tests
cargo test

# Run benchmarks (rer-version only)
cargo bench

# Run the resolver example (requires data_set_private/)
cargo run -p examples --example new_solver

# Run the benchmark comparison (requires data_set_private/)
cargo run -p examples --example rez_benchmark

# Test a specific crate
cargo test -p rer-version
cargo test -p rer-resolver
```

## Dependencies

| Crate | Version | Role |
|---|---|---|
| `pubgrub` | 0.3.0 | Version solving algorithm (DependencyProvider trait, resolve()) |
| `version-ranges` | 0.1 | `Ranges<V>` type for version set operations (extracted from pubgrub) |
| `regex` | 1.10 | Version/requirement tokenization |
| `lazy_static` | 1.5 | Compiled regex caching |
| `serde` + `serde_json` | 1.0 | Package repo JSON serialization |
| `rayon` | 1.10 | Parallel candidate selection (experimental) |
| `uuid` | 1.9 | Root package identity in solver |
| `clap` | 4.4 | CLI argument parsing (rer binary) |
| `criterion` | 0.5 | Benchmarks (dev-dependency of rer-version) |

### Planned (not yet in workspace)

| Crate | Role |
|---|---|
| `pyo3` | Python ↔ Rust bridge (Phase 3) |
| `maturin` | Build Python wheels from Rust (Phase 3) |

## Reference: rez Solver Behavior

When verifying solver correctness, the authoritative reference is rez's source:
- Solver: `src/rez/solver.py` — especially `_ResolvePhase`, `_PackageScope`, `Solver.solve()`
- Version: `src/rez/version/_version.py` — `Version`, `VersionRange`, `AlphanumericVersionToken`
- Requirement: `src/rez/version/_requirement.py` — `Requirement`, `RequirementList`
- Tests: `src/rez/tests/test_solver.py`, `src/rez/tests/test_version.py`
- Docs: https://rez.readthedocs.io/en/stable/package_definition.html