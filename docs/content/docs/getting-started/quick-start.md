+++
title = "Quick Start"
description = "Install rer and run your first resolve from Python or Rust."
date = 2026-05-15T08:20:00+00:00
updated = 2026-05-15T08:20:00+00:00
draft = false
weight = 20
sort_by = "weight"
template = "docs/page.html"

[extra]
lead = "Install rer and run your first resolve from Python or Rust."
toc = true
top = false
+++

## From Python

The Python package is published to PyPI as `pyrer` (the name `rer` is
taken on PyPI). The wheel is a stable-ABI build that works on
**CPython 3.9+** on Linux, macOS, and Windows.

### Install

```bash
pip install pyrer
```

### Resolve

```python
import pyrer

packages = [
    pyrer.PackageData("app", "1.0.0", requires=["lib-2"]),
    pyrer.PackageData("lib", "1.0.0"),
    pyrer.PackageData("lib", "2.0.0"),
]

result = pyrer.solve(["app"], packages)
print(result.status)        # "solved"
for v in result.resolved_packages:
    print(v.name, v.version, v.variant_index, v.uri)
# app 1.0.0 None app/1.0.0/package.py
# lib 2.0.0 None lib/2.0.0/package.py
```

`pyrer.solve(package_requests, packages)` takes:

- `package_requests` — a list of rez-style requirement strings, e.g.
  `["python-3", "maya-2024"]`.
- `packages` — either:
  - a `list[pyrer.PackageData]` (preferred), or
  - a JSON string mapping `name -> version -> {"requires": [...],
    "variants": [[...]]}` (kept for legacy callers).

  The data is the host (rez)'s already-loaded packages; `rer` does not
  read the filesystem itself.

It returns a `SolveResult` with:

- `status` — `"solved"`, `"failed"` (a real resolve conflict), or
  `"error"` (bad input).
- `resolved_packages` — list of `pyrer.ResolvedVariant` objects with
  `name`, `version`, `variant_index`, `requires`, and a rez-shaped
  `uri` (`"name/version/package.py[idx]"`).
- `resolved` — same resolution as `(name, version, variant_index)`
  tuples, kept for callers that prefer the simpler shape.
- `failure_description` — populated on `"failed"` / `"error"`.
- `solve_time_ms`, `num_iterations` — timing and solver-step counts.

Failures and bad input are reported via `status`, never as Python
exceptions (except `TypeError` if `packages` is the wrong type).

## From Rust

Add the resolver crate to your `Cargo.toml`:

```toml
[dependencies]
rer-resolver = "0.1.0-rc.6"
```

Then call the solver against an in-memory repository:

```rust
use std::collections::HashMap;
use std::rc::Rc;
use rer_resolver::PackageData;
use rer_resolver::rez_solver::{Requirement, Solver, SolverStatus};

let mut repo: HashMap<String, HashMap<String, PackageData>> = HashMap::new();

// app-1.0.0 requires lib-2; lib has versions 1.0.0 and 2.0.0
repo.insert(
    "app".into(),
    [(
        "1.0.0".to_string(),
        PackageData { requires: vec!["lib-2".into()], variants: vec![] },
    )]
    .into_iter()
    .collect(),
);
repo.insert(
    "lib".into(),
    [
        ("1.0.0".to_string(), PackageData::default()),
        ("2.0.0".to_string(), PackageData::default()),
    ]
    .into_iter()
    .collect(),
);

let reqs = vec![Requirement::parse("app")];
let mut solver = Solver::new(reqs, Rc::new(repo)).unwrap();
solver.solve();

assert_eq!(solver.status(), SolverStatus::Solved);
// Resolves to app-1.0.0 + lib-2.0.0
```

## Building from source

If you need to hack on rer, clone the repo and build it with `maturin`
inside a virtualenv:

```bash
git clone https://github.com/doubleailes/rer.git
cd rer

python -m venv .venv && . .venv/bin/activate
pip install maturin pytest

cd crates/rer-python && maturin develop   # builds + installs `pyrer`
cd ../..
pytest tests/test_differential.py -v      # 58 cases
```

For the full Rust-side test suite:

```bash
cargo build
cargo test
```

The `test_rez_benchmark` integration test (the 188-case rez differential)
is `#[ignore]`d because the release run takes several minutes. It needs
the `rez` git submodule:

```bash
git submodule update --init
python scripts/prepare_benchmark_data.py
cargo test --release -p rer-resolver --test test_rez_benchmark -- --ignored
```
