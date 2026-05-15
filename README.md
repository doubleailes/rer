# rer

![rer.logo](https://github.com/doubleailes/rer/blob/main/docs/content/images/res_v01.png)

**r.e.r** stands for the French phrase *"Rez En Rust"* — a pun on the Parisian
suburban train, the [réseau express régional d'Île-de-France](https://en.wikipedia.org/wiki/R%C3%A9seau_Express_R%C3%A9gional).

## What it is

`rer` is a Rust reimplementation of the solver hotpath of
[rez](https://github.com/AcademySoftwareFoundation/rez), the VFX/animation
package manager.

The solver is a **faithful port of rez's own phase-based backtracking solver**
(`rez/src/rez/solver.py`) — not a different algorithm dressed up to look
similar. It reproduces rez's mechanics exactly: weak (`~`) and conflict (`!`)
requirement semantics, variant selection order, the extract / intersect /
reduce / split cycle, and implicit backtracking. The goal is output that
matches rez **1:1**, not merely "a valid resolve".

The end goal is a **hybrid integration**: a Rust library callable from Python
via PyO3 that accelerates rez resolves while leaving the rest of rez untouched.

### Status

- ✅ **Solver** — complete and rez-faithful.
- ✅ **Validated 1:1** against rez's bundled 188-case benchmark dataset on
  **solve status** and the **resolved `(name, version)` set** — the
  differential test currently does not enforce variant-index parity, only
  same packages at the same versions.
- ✅ **Fast** — see the [Benchmark](#benchmark) section below for a local
  apples-to-apples measurement against rez 3.3.0.
- ✅ **Python bridge** — `pyrer.solve(...)` runs the ported solver (the
  `rer-python` crate ships to PyPI as `pyrer` — `rer` is taken;
  `pip install pyrer`, `import pyrer`).

## Workspace

A virtual Cargo workspace — use `-p <crate>` for crate-specific commands.

| Crate | Role |
|---|---|
| **`rer-version`** | `RerVersion` (rez token ordering) and `VersionRange` (rez range semantics over [`version-ranges`](https://crates.io/crates/version-ranges)). |
| **`rer-resolver`** | The solver. `rez_solver` is the rez port — `Solver`, `ResolvePhase`, `PackageScope`, the variant structures, `Requirement`/`RequirementList`. `PackageData` is the in-memory unit of the package repository. |
| **`rer-python`** | PyO3 bridge (Python import name `pyrer`, PyPI distribution `pyrer`), built into wheels by maturin. |
| **`examples`** | `rez_benchmark_dataset` — a timing report. |

`rer` works on an **in-memory package repository** (`family → version →
{requires, variants}`); it does not read the filesystem itself — the host
(rez) hands the loaded data in.

## Using it from Rust

```rust
use std::rc::Rc;
use rer_resolver::rez_solver::{Requirement, Solver, SolverStatus};
use rer_resolver::PackageData;

let mut repo = std::collections::HashMap::new();
// app-1.0.0 requires lib-2; lib has 1.0.0 and 2.0.0
repo.insert("app".into(), [("1.0.0".to_string(), PackageData {
    requires: vec!["lib-2".into()], variants: vec![],
})].into_iter().collect());
repo.insert("lib".into(), [
    ("1.0.0".to_string(), PackageData::default()),
    ("2.0.0".to_string(), PackageData::default()),
].into_iter().collect());

let reqs = vec![Requirement::parse("app")];
let mut solver = Solver::new(reqs, Rc::new(repo)).unwrap();
solver.solve();
assert_eq!(solver.status(), SolverStatus::Solved);
// resolves to app-1.0.0 + lib-2.0.0
```

## Using it from Python

```bash
python -m venv .venv && . .venv/bin/activate
pip install maturin
cd crates/rer-python && maturin develop
```

```python
import pyrer

packages = [
    pyrer.PackageData("app", "1.0.0", requires=["lib-2"]),
    pyrer.PackageData("lib", "1.0.0"),
    pyrer.PackageData("lib", "2.0.0"),
]
result = pyrer.solve(["app"], packages)

print(result.status)                         # "solved"
for v in result.resolved_packages:
    print(v.name, v.version, v.variant_index, v.uri)
# app 1.0.0 None app/1.0.0/package.py
# lib 2.0.0 None lib/2.0.0/package.py
```

`solve()` reports failures and bad input via `result.status`
(`"solved"` / `"failed"` / `"error"`), never as a Python exception
(except a `TypeError` if `packages` isn't a list of `PackageData` or a
JSON string).

`pyrer.solve` also still accepts the original JSON-string form
(`pyrer.solve(["app"], json.dumps(repo_dict))`) for callers that
prefer it; the resolution is identical.

### Wiring `pyrer` behind `rez`

`pyrer` only does the solve; `rez` still does package discovery, env
construction, and the whole `ResolvedContext` lifecycle. To plug
`pyrer` in behind a normal `rez env` / `ResolvedContext` flow:

1. Walk rez's package paths into `pyrer.PackageData` objects — once
   per process, reusable across many solves:

   ```python
   import pyrer
   from rez.packages import iter_package_families

   def build_pyrer_packages(package_paths):
       for fam in iter_package_families(paths=package_paths):
           for pkg in fam.iter_packages():
               yield pyrer.PackageData(
                   name=fam.name,
                   version=str(pkg.version),
                   requires=[str(r) for r in (pkg.requires or [])],
                   variants=[[str(r) for r in v] for v in (pkg.variants or [])],
               )
   ```

2. Call `pyrer.solve(requests, list(build_pyrer_packages(paths)))`
   instead of `rez.solver.Solver.solve()`.

3. Read `result.resolved_packages` — each entry already has `.name`,
   `.version`, `.variant_index`, `.requires` and a rez-shaped `.uri`.
   If a downstream consumer needs the full rez `Variant` (for
   `commands`, `tools`, etc.), look it up with
   `rez.packages.get_package(rv.name, rv.version).get_variant(rv.variant_index or 0)`.

The
[Wiring `pyrer` into `rez`](https://doubleailes.github.io/rer/docs/getting-started/rez-integration/)
guide has the full walkthrough — a minimal monkey-patch of
`Resolver._solve`, a fallback for configs `pyrer` doesn't model yet
(`intersection_priority`, `@early` / `@late` requires, custom
orderers / filters), and a sanity-check loop for diffing `pyrer`
against rez on your own repo.

## Building & testing

```bash
cargo build                          # build all crates
cargo test                           # unit + integration tests
cargo bench                          # benchmarks (rer-version)
```

### The rez benchmark

The 1:1 differential test against rez's bundled benchmark is `#[ignore]`d (the
full release run takes several minutes). It needs the `rez` git submodule:

```bash
git submodule update --init
python scripts/prepare_benchmark_data.py     # -> data_set/benchmark_*.json
cargo test --release -p rer-resolver --test test_rez_benchmark -- --ignored
cargo run  --release -p examples --example rez_benchmark_dataset   # timing report
```

The `benchmark` CI workflow runs this on every PR touching the resolver.

## Benchmark

`rer` and `rez` are timed on the **same machine** running the **same
188-case workload** (`rez/src/rez/data/benchmarking/`). Both runs use one
core and the default solver configuration.

### Reference run, 2026-05-15

```text
machine: Intel(R) Xeon(R) CPU E5-2699 v4 @ 2.20 GHz, 32 cores
OS:      Linux 5.14.0 (glibc 2.34)
```

The headline comparison uses **CPython 3.13** for `rez` — that's the
fastest current CPython and is the realistic target for modern rez
deployments. We include a CPython 3.9 row too because rez's solver is
sensitive to interpreter speed and a lot of VFX studios still run 3.9.

| Implementation                          | Total (188 cases) | Mean / case | Median | p95 | Max |
|---|---:|---:|---:|---:|---:|
| **rez 3.3.0** on CPython 3.13           | 221.43 s | 1 177 ms |   587 ms | —      | 5 770 ms |
| **rer 0.1.0-rc.6** (no Python in loop)  |  11.35 s |    60 ms |    30 ms | 181 ms |   247 ms |
| **speedup vs rez on 3.13**              |  **19.5×** | **19.5×** | **19.9×** | —      | **23.4×** |
| *(rez 3.3.0 on CPython 3.9 — for ref)*  | *405.17 s* | *2 152 ms* | *1 162 ms* | —    | *9 399 ms* |

`rer` solved 187 of 188 requests with the same `(name, version)` set as `rez`
on every solve; the one failed request fails on both sides. The differential
test confirms this on every PR (`cargo test --release -p rer-resolver --test
test_rez_benchmark -- --ignored`).

### How to reproduce

```bash
# 1. rez 3.3.0 (vendored as a submodule), on CPython 3.13 via uv
git submodule update --init
uv python install 3.13
uv venv --python 3.13 /tmp/rez-bench-venv
uv pip install --python /tmp/rez-bench-venv/bin/python ./rez
/tmp/rez-bench-venv/bin/rez-benchmark --out /tmp/rez-bench-out
cat /tmp/rez-bench-out/summary.json   # records hardware + total_run_time

# 2. rer, same machine
python3 scripts/prepare_benchmark_data.py   # fixtures from the rez submodule
cargo run --release -p examples --example rez_benchmark_dataset
```

### Historical context

`rez/metrics/benchmarking/artifacts/2022.11.16-3.7-2.112.0/summary.json`
records a published rez run on a 2-core Azure VM in 382.68 s — useful as
upstream context, but **not directly comparable** to a `rer` run on
different hardware. The numbers above are the apples-to-apples comparison.

### Where the speedup comes from

The solver itself is a faithful port; the wall-clock difference is a Rust
implementation of an algorithm that was tuned for Python. Notable
contributions, in rough order of impact on this benchmark:

| | Change | Effect |
|---|---|---|
| 1 | Variant cache shared across solves | -20 % |
| 2 | `is_subset` → length compare on the extract hot-path | -30 % |
| 3 | Pre-filter pending reduction pairs by `fam_requires` | -25 % |
| 4 | `Rc<str>` for package family names | -15 % |
| 5 | `Rc<Ranges>` inside `VersionRange` | -11 % |
| 6 | `mimalloc` as the global allocator in the bench binary | -13 % |
| 7 | Skip `extract` on scopes known to be exhausted | -5 % |

Compounding from 43.0 s (post-port baseline) down to 11.35 s.

## License

See [LICENSE](LICENSE).
