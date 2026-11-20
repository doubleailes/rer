+++
title = "Package loading in Rust"
description = "rer loads rez package definitions and lists filesystem repositories in Rust, reproducing rez's data exactly or handing the package back to rez's Python loader."
draft = false
weight = 35
sort_by = "weight"
template = "docs/page.html"

[extra]
lead = "rer loads rez package definitions and lists filesystem repositories in Rust. It either reproduces rez's data exactly or hands the package back to rez's Python loader. It never guesses."
toc = true
top = false
+++

## What changed

The four-field static parser (see the
[fast parser RFC](../fast-package-py-parser/)) fed only the solver. This
step moves *package loading* itself into Rust:

| Layer | Crate | Reproduces |
|---|---|---|
| Definition file → data dict | `rer-package` (`definition`) | `rez.serialise.load_py` / `load_yaml` |
| Repository listing + `_load` post-processing | `rer-repository` | `rezplugins/package_repository/filesystem.py` |
| Python bridge, opt-in | `pyrer` | `PackageRepository`, `solve(package_repository=...)`, `load_package_file` |

The solver and version crates are unchanged.

## The contract: reproduce or hand back

Every entry point returns either rez's exact result or an explicit
"needs Python" value:

- `rer_package::definition::LoadOutcome::{Static, NeedsPython}` per file;
- `rer_repository::PackageOutcome::{Static, NeedsPython}` per package;
- `rer_repository::FamilyLoad::NeedsPython` for a whole family.

`Static` means *the same dict rez builds*: same keys, same key order,
same Python types, and for `commands()` / `@late` functions the same
`SourceCode` state (`source`, `funcname`, `decorators`,
`eval_as_function`). rez never calls `SourceCode.func` after loading (it
compiles `.source`), so this state is all rez ever uses.

### Handed back to rez

| Granularity | Cases |
|---|---|
| Package | `@early`, imports, control flow, classes, `def preprocess`, non-literal expressions, bytes / f-strings / sets / big ints, `\N{}` escapes, tab indentation, non-UTF-8 sources, YAML anchors / tags / timestamps / merge keys / non-decimal ints, non-ASCII YAML (locale-dependent), `format_version > 2`, old-format release metadata, late-bound `requires` / `variants` (solver path), anything rez itself raises on |
| Family | combined `<name>.py` family files, unversioned packages, version directories rez can't parse, version directories that compare equal |

## How static files are loaded

`package.py` is tokenized with CPython's token rules (INDENT / DEDENT /
NL / NEWLINE / COMMENT positions included). Only these top-level
statements are accepted: literal assignments, `def` (undecorated, or
`@late()` / `@include(...)`), `with scope("…") [as x]:` blocks of literal
assignments, string statements and `pass`. The exec globals are then
post-processed the way `load_py` does it: dict insertion order on
reassignment, `scope` capture and restore (including "equal value is not
captured"), rex / `@late` functions turned into `SourceCode`, other
functions dropped, `__` keys trimmed (also in nested dicts).

Function source text ports `inspect.BlockFinder` and rez's
dedent / comment-realignment code line for line.
`scripts/crossversion_definitions.py` checks that rez gives the same
result on CPython 3.8–3.13 (rez 3.3 requires ≥ 3.8): 0 differences over
3,123 files.

`package.yaml` uses `yaml-rust2` for syntax and PyYAML's YAML 1.1
resolvers (in PyYAML's order) for scalars.

## Differential results

| Corpus | Files / checks | Blockers |
|---|---:|---:|
| `tests/fixtures/definitions` (edge cases) | 62 files | 0 |
| rez's own test data | 88 files | 0 |
| rez benchmark repo (`packages.tar.gz`) | 18,226 files | 0 |
| fuzzed `package.py` (`scripts/fuzz_definitions.py`) | 12,000 files | 0 |
| fuzzed `package.yaml` | 24,000 files | 0 |
| repository listing + `_data`, fixtures + rez test repos + benchmark repo | 3,900+ family/range checks, 26,500+ packages | 0 |
| benchmark resolves, `pyrer.solve(package_repository=...)` vs rez solver | 188 | 0 |
| benchmark resolves through `ResolvedContext` (patched rez), variant handles | 188 / 60 | 0 |

A *blocker* is a file where rer returned data that differs from rez's,
or where rer returned data and rez raised.

## Profile: before and after

`scripts/profile_rez_resolves.py`, the 188 requests of rez's benchmark
against its 18,226-package repo, `ResolvedContext` with
`package_repository_manager.clear_caches()` before each resolve; local
disk, warm page cache, no memcached; mean of two runs (runs within 2%).

| Bucket (s) | `use_rer_solver` (before) | `+ use_rer_package_loader` (after) |
|---|---:|---:|
| **Wall** | **69.6** | **37.0** |
| Python `load_family` (rez `iter_packages`, static parse, PackageData) | 39.3 | 0 |
| — of which rez `iter_packages` | 30.0 | — |
| — of which batched static parse | 5.7 | — |
| `pyrer.solve` excluding Python callbacks (solve + Rust loading) | 13.2 | 23.2 |
| Post-solve rez Variant construction | 14.6 | 11.7 |
| Rest of `ResolvedContext` | 2.4 | 2.1 |
| Resolves solved (rez solver: 187) | 186 | 187 |

The Rust-loading share of the 23.2 s is inferred (≈ 10 s: 23.2 − 13.2),
not measured separately. Caches are cleared before every resolve, so
each of the 387,168 package loads in the run reads and parses its file.
`num_loaded_packages` is identical in both modes on every request that
solves in both.

## Wiring into rez

`scripts/rez-patches/0001-rer-package-loader.patch` (for the patched rez
on top of `use_rer_solver`) adds `use_rer_package_loader`. When it is on
and the resolve has no package filter, no user package-load callback, and
only filesystem repositories:

- `RerSolver` passes a per-process `pyrer.PackageRepository`; rez's loader
  is the per-family / per-package fallback;
- `FileSystemPackageResource._load` tries `pyrer.load_package_file` first,
  so every rez consumer of package data benefits;
- `package_repository_manager.clear_caches()` also clears rer's caches.

## Caching

In-process only, mirroring rez: listings and loaded definitions are
cached per repository with no invalidation until `clear_caches()`, like
rez's `lru_cache`s and resource pool. A persistent cache (keyed by
realpath, size, mtime, file id, loader version, settings) is not
implemented yet.

## Not verified

- Windows and macOS (case-insensitive path handling is implemented from
  rez's code but never run; Windows has no stable file id in std).
- Network filesystems / cold caches; all numbers are local disk, warm.
- A real studio corpus: the benchmark repo is synthetic and fully static.
- rez's memcached path (the hook bypasses memcached for definitions rer
  serves).
