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
- ✅ **Validated 1:1** — against rez's bundled 188-case benchmark dataset, every
  resolve matches rez's own recorded result exactly (same status, same package
  set).
- ✅ **Fast** — on that benchmark, on one machine, `rer` resolves all 188
  requests in ~44 s versus ~206 s for rez 3.3.0 (`rez benchmark`). Correctness
  is version-independent; the timing is same-machine context, not a lab claim.
- ✅ **Python bridge** — `rer_solver.solve(...)` runs the ported solver.
- 🚧 **CLI** (`rer`) — a placeholder; the build/bind/env subcommands are future
  work.

## Workspace

A virtual Cargo workspace — use `-p <crate>` for crate-specific commands.

| Crate | Role |
|---|---|
| **`rer-version`** | `RerVersion` (rez token ordering) and `VersionRange` (rez range semantics over [`version-ranges`](https://crates.io/crates/version-ranges)). |
| **`rer-resolver`** | The solver. `rez_solver` is the rez port — `Solver`, `ResolvePhase`, `PackageScope`, the variant structures, `Requirement`/`RequirementList`. `PackageData` is the in-memory unit of the package repository. |
| **`rer-python`** | PyO3 bridge (module name `rer_solver`), built into wheels by maturin. |
| **`rer`** | `clap` CLI binary (placeholder). |
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
import json, rer_solver

repo = {
    "app": {"1.0.0": {"requires": ["lib-2"], "variants": []}},
    "lib": {"1.0.0": {"requires": [], "variants": []},
            "2.0.0": {"requires": [], "variants": []}},
}
result = rer_solver.solve(["app"], json.dumps(repo))
print(result.status)    # "solved"
print(result.resolved)  # [("app", "1.0.0", None), ("lib", "2.0.0", None)]
```

`solve()` reports failures and bad input via `result.status`
(`"solved"` / `"failed"` / `"error"`), never as a Python exception.

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

## License

See [LICENSE](LICENSE).
