+++
title = "RFC — A blazing-fast Rust `package.py` parser"
description = "Forward-looking design for a Rust parser that extracts the four solver-relevant fields from a rez package.py without invoking Python. Built to compose with the load_family lazy-discovery hook on cold-cache integrations."
draft = false
weight = 30
sort_by = "weight"
template = "docs/page.html"

[extra]
lead = "Forward-looking design for a Rust parser that extracts the four solver-relevant fields from a rez package.py without invoking Python. Built to compose with the load_family lazy-discovery hook on cold-cache integrations."
toc = true
top = false
+++

> **Status: Stage 1 complete, Stage 2 scaffolded.** Survey tool at
> `scripts/survey_package_py.py`; Rust parser crate at
> `crates/rer-package/`. Stage 1 numbers (Fortiche, May 2026) inline
> below.

## Stage 1 result — Fortiche, May 2026

Run on `/thierry/rez/pkg` (the Fortiche-on-CIFS rez repo):

| | Count | % |
|---|---:|---:|
| `package.py` files surveyed | 6,439 | 100% |
| **Fast-parseable** | **5,982** | **92.9%** |
| Not fast-parseable | 457 | 7.1% |

Non-fast-parseable breakdown (files can match multiple buckets):

| Pattern | Count | % of total |
|---|---:|---:|
| `dynamic-requires` (`@early` / `@late` on `requires`) | 352 | 5.5% |
| `imports` (load-bearing `import` statements) | 96 | 1.5% |
| `missing-version` (mostly rez's own test fixtures) | 85 | 1.3% |
| `missing-name` (mostly rez's own test fixtures) | 75 | 1.2% |
| `top-level-classdef` | 54 | 0.8% |
| `unrecognised-raise` | 2 | 0.0% |

**Decisive finding**: the dominant *seeming* failure pattern from a
naive survey, `top-level-with` (2,245 files, 34.9% of the corpus), is
**100% rez's declarative `with scope("config")` DSL** — every one of
the 2,245 files matched. That body only writes attributes of the
`as`-name (config object) and never touches solver fields, so it is
solver-irrelevant — the parser treats it the same way it treats
`def commands(...)`. Including this single extension lifts the
accept rate from a marginal 58.6% to a green-light 92.9%.

Well past the 70% **PROCEED** threshold from the original RFC.

## Motivation

`pyrer`'s solver is already roughly 34× faster than `rez`'s on the
188-case differential benchmark. Real `rez env` invocations are no
longer bottlenecked on the solve — they are bottlenecked on what
surrounds it:

1. Python interpreter startup (~200–300 ms per process).
2. **Package discovery — opening, reading, and AST-evaluating each
   `package.py` rez decides to inspect.** This is the big remaining
   cost on cold-cache invocations.
3. The solve itself (~tens of ms on `pyrer`).
4. Environment construction (Rex evaluation, PATH munging, shell
   hooks).

Issue #86 added the `load_family` callback so the solver only asks
the host to load families it actually needs — that addresses the
"how many" axis. This RFC addresses the "how fast each one" axis:
loading a `package.py` currently means a full Python compile + exec,
which can run into the milliseconds per file on a warm cache and
adds up fast across a wide BFS or a CI batch.

A Rust parser that extracts the four solver-relevant fields without
invoking the Python interpreter has the potential to drop per-file
parse cost from milliseconds to tens of microseconds for the static
majority of `package.py` files. Combined with `load_family`, it
attacks both the count and the cost of the discovery phase.

## What `package.py` actually is

A `rez` `package.py` is **arbitrary Python**. The solver only reads
four fields:

- `name` — string
- `version` — string
- `requires` — list of rez-requirement strings
- `variants` — list of lists of rez-requirement strings

But a real-world `package.py` can also carry, with varying
frequency:

- `description`, `authors`, `tools`, `tests`, `help` — irrelevant to
  the solve.
- `commands()`, `pre_commands()`, `post_commands()` — function bodies
  that affect *runtime* environment, not the solve.
- `build_command`, `build_system` — irrelevant to the solve.
- **`@early()` / `@late()` decorated functions on `requires` /
  `variants`** — *these are dynamic and do affect the solve.*
- Top-level `if/else` chains on env vars (`if config.studio_mode:
  requires = […]`) — also dynamic relative to the solve.
- Top-level `import` statements with load-bearing side effects.

The fast parser only needs to handle the case where the four solver
fields are **literal assignments**. Everything else falls back to
`rez`'s evaluator, which already exists and is correct.

## Scope

### In scope (fast path)

| Statement | Action |
|---|---|
| `name = "..."` (string literal) | Extract |
| `version = "..."` (string literal) | Extract |
| `requires = ["str", "str", …]` (list of string literals) | Extract |
| `variants = [["str", …], …]` (list of lists of string literals) | Extract |
| `def commands(...)`, `def pre_commands(...)`, `def post_commands(...)`, `def tools(...)` (function body) | Ignore — not solver-relevant |
| Top-level assignments to non-solver fields (`description`, `authors`, `tools`, `tests`, `help`, `build_command`, `build_system`, etc.) | Ignore |
| Top-level docstring | Ignore |

### Out of scope (bail to `rez`)

| Pattern | Why we bail |
|---|---|
| `def requires(...)` / `def variants(...)` with `@early` / `@late` | Solver-relevant value is dynamic |
| Top-level `if/else`, `try/except`, `for` | Can't statically know which branch wins |
| `import` / `from ... import` | May have load-bearing side effects |
| Function calls assigning to a solver field (`requires = make_requires(...)`) | Not statically resolvable |
| Any other expression we don't recognise | Conservative bail |

**The bias is hard toward bailing.** A false positive (parsing a file
the fast path shouldn't have handled) produces a different `requires`
than rez, which means different resolves, which is a silent
correctness regression. The slow path through rez always exists; the
fast path is opt-in coverage. We accept low coverage with zero
divergence over high coverage with any divergence.

## Architecture

### New crate: `rer-package`

```text
crates/rer-package/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── parser.rs      # AST walk + literal extraction
│   └── classify.rs    # bail-or-extract decisions
└── tests/
    ├── static_fixtures/    # known-fast-parseable .py files
    ├── dynamic_fixtures/   # known-bail .py files
    └── corpus/             # large real-world sample, diff against rez
```

Depends on `rustpython-parser` (well-maintained, parses to AST, no
runtime dependency on a Python install). Returns:

```rust
pub fn parse_static_package_py(source: &str) -> Option<PackageData>
```

`None` means the file is not statically parseable; the caller falls
back to rez. `Some(data)` means the four fields were all extracted
as literals; the caller can skip `rez.Package` evaluation entirely.

### PyO3 binding

Single function on `pyrer`:

```python
pyrer.parse_static_package_py(source: str | bytes) -> Optional[pyrer.PackageData]
```

About twenty lines of glue. The integration site is the
`load_family` callback in the rez shim:

```python
def load_family(name):
    out = []
    for pkg_path in _find_package_files(name, paths=PACKAGE_PATHS):
        with open(pkg_path) as f:
            source = f.read()
        # Fast path: try the Rust parser first.
        pd = pyrer.parse_static_package_py(source)
        if pd is None:
            # Bail to rez's evaluator for the dynamic case.
            pkg = _rez_package_from_file(pkg_path)
            pd = pyrer.PackageData.from_rez(pkg)
        out.append(pd)
    return out
```

The shim composes the two — `load_family` decides *which* files to
read; `parse_static_package_py` decides *how fast* to read each one.

## Build order

### Stage 1 — Corpus survey (~2 days)

Build `rer-stat-package-py`: a tool that walks a directory tree of
`package.py` files and classifies each one into:

| Category | Meaning |
|---|---|
| `fast-parseable` | All four solver fields are literal assignments; no disqualifying top-level statements. |
| `dynamic-requires` | `requires` is `@early`/`@late` or assigned conditionally. |
| `dynamic-variants` | Same, for `variants`. |
| `top-level-if` | A top-level `if/else` we'd have to bail on. |
| `imports` | Has `import` statements. |
| `other` | Anything else that disqualifies the fast path. |

Reports counts, percentages, and example file paths per bucket.

**Run this against Fortiche's actual studio repo.** The output is
the go/no-go signal for Stage 2:

- ≥ 70% fast-parseable: proceed. The fast path covers the typical
  case; the engineering ROI is real.
- 40–70%: marginal. Worth a discussion about which patterns to
  expand coverage to and whether the complexity is worth it.
- < 40%: don't build it. The slow-path fallback would dominate;
  the fast path saves work in the minority case. The memcache
  alternative below is the smarter bet.

Stage 1 is the cheapest experiment that produces the number this
project needs.

### Stage 2 — Parser + binding + differential test (~1–2 weeks)

1. Implement `rer_package::parse_static_package_py` against
   `rustpython-parser`.
2. Hand-curate ~30 fixture files (≥ 15 static, ≥ 15 dynamic across
   every disqualifying pattern). Unit-test both arms.
3. PyO3 binding on `pyrer.parse_static_package_py`.
4. **Differential test harness**: for every file in the corpus where
   the fast parser returns `Some(data)`, also load the file through
   rez's `Package` and compare the four fields. **Any mismatch is a
   release blocker**, exactly like the 188-case rez solver
   differential. This is the safety net for the "bias toward bail"
   policy.

### Stage 3 — Shim wiring + end-to-end benchmark (~3–5 days)

1. Document the `load_family` integration pattern (above).
2. End-to-end benchmark: a real `rez env` invocation against a
   representative repo, three configurations:
   - Eager BFS (today's shim baseline)
   - `load_family` only (issue #86 today)
   - `load_family` + `parse_static_package_py` (this RFC)

   Report wall-clock for cold and warm cache.

## Honest forecast — superseded by Stage 3 measurement

The original RFC predicted 2–50× wins by replacing rez's
"compile + exec" with Rust AST parsing. Stage 3's measurement
against real Fortiche files showed something smaller and worth
naming clearly.

## Stage 3 result — Fortiche, May 2026

Run with `scripts/bench_package_py_parser.py --corpus /thierry/rez/pkg
--samples 100 --iters 30 --with-rez`, against the live Fortiche-on-CIFS
repo, against rez 3.3.0.

**Apples-to-apples full-load comparison** (what the rez-integration
shim actually pays per file):

| Path | μs / file | Ratio |
|---|---:|---:|
| `open + read + parse_static_package_py` | **1,533** | — |
| `DeveloperPackage.from_path + from_rez` (rez) | **2,632** | — |
| **Speedup** | | **1.7×** |

For context, the in-memory (no I/O) breakdown:

| Path | μs / file |
|---|---:|
| `parse_static_package_py(source)` (Rust AST parse) | 1,990 |
| `from_rez(fake_pkg)` (attribute walk, lower bound) | 12.7 |
| `from_rez(real rez Package)` (post-load, just walk) | 15.0 |
| file read alone (open + read) | 18.7 |

**What this says** — honestly:

- The Rust parser is the bottleneck of its own path. `rustpython-parser`
  builds the full AST of every file even though we only read four
  fields; that's the ~2 ms.
- rez 3.3.0's `DeveloperPackage.from_path` is already doing something
  reasonably fast (~2.6 ms full load). The Rust parser isn't competing
  against `compile + exec` of arbitrary Python (which the RFC
  assumed); it's competing against rez's own AST-based loader.
- The 1.7× speedup translates to roughly **1.1 ms saved per file
  loaded**. Over a resolve that loads 50 families (typical with the
  `load_family` hook from #86), that's ~55 ms saved. Real but
  modest — same order of magnitude as the per-resolve wins from
  `from_strings` (#88) and `load_family` (#86), not transformative
  on its own.

## Where the next 10× lives

If the 1.7× speedup is the headline, then `rustpython-parser` is
the obvious optimisation target. A hand-rolled lexer that scans
just for `^name = "..."`, `^version = "..."`, `^requires = [...]`,
`^variants = [[...]]` at module scope — no AST, no full parse —
should land in the 50–200 μs/file range. That would lift the
full-load comparison to ~5–15× over rez and would change the
project economics meaningfully.

Two reasonable next experiments, in order of effort:

1. **Switch to `ruff_python_parser` if a usable crates.io publish
   exists.** Ruff's parser is much faster than `rustpython-parser`
   but not officially published as a standalone crate (only
   vendored forks like `littrs-ruff-python-parser`). Worth a
   spike.
2. **Hand-rolled lexer.** Treat the parser as a glorified four-rule
   regex/scanner. Bias hardest toward bailing. Much smaller crate;
   the 20 unit tests we have for the AST-based version mostly
   transfer.

Both unlock further savings on top of the 1.7× we already have.

## When this version is worth shipping anyway

- If the rez shim is going to call `load_family` 50+ times per
  resolve, the 55 ms / resolve saving is real artist-perceptible
  latency.
- If Fortiche's shared memcache stores raw `package.py` bytes (not
  parsed `PackageData`), this parser displaces work that would
  otherwise be repeated on every `rez env` invocation. The memcache
  alternative subsumes this.
- If `rustpython-parser` is already a transitive dep of something
  else in the dev environment, the build cost is shared.

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| **Silent correctness regression** — the fast parser accepts a file it shouldn't | Bias hard toward bailing in `classify.rs`. Run the differential test on every file the fast parser claims. Treat any mismatch as a release blocker. |
| **Maintenance burden** — `rustpython-parser` is a Python AST library; track upstream Python syntax changes | Pin to a known-good version. Studio `package.py` files don't typically use bleeding-edge syntax. |
| **Coverage drift** — over time studios add patterns the parser doesn't handle | The fast path is opt-in. Coverage drift means the slow path runs more often, not that correctness breaks. We can extend coverage when patterns become common. |
| **Stage 1 says "don't build it"** | We've spent two days on a survey and now know the workload. Pivot to the memcache route below or accept the status quo. Cheap pivot point. |

## Considered alternatives

### Parsed-package cache on top of the shared memcache

Instead of parsing `package.py` fast, parse once via rez and cache
the four-field result in Fortiche's existing shared memcache, keyed
by `(repo_id, family, version, mtime)`. Subsequent reads across the
studio are sub-millisecond regardless of whether the file is
static or dynamic.

Tradeoffs:

- **Pro**: no novel parser, no AST-classification edge cases, reuses
  infrastructure already in place. Lower risk, faster to ship.
- **Pro**: works for dynamic packages too (cache stores the
  *evaluated* fields).
- **Con**: first hit anywhere in the studio still pays full rez
  evaluation. The Rust parser is "always fast"; the cache is "fast
  after first studio-wide hit".
- **Con**: requires invalidation on `package.py` change (mtime check
  is cheap but adds complexity).

The user's preference is to explore the Rust parser. The cache is
worth re-evaluating after Stage 1 — if the static-parseable fraction
is lower than expected, the cache pays off more reliably with less
risk.

## Concrete next step

Open a draft PR with `rer-stat-package-py` only: the corpus
classifier walker. No parser yet. The deliverable is a small CLI
that takes a path and prints a histogram of categories. Two days
of work; the output decides whether Stages 2–3 happen.

A reasonable acceptance criterion for the survey tool itself:

- Recognises every pattern listed in the [Scope](#scope) section.
- Reports per-category counts and percentages.
- Outputs example file paths for each bucket (for hand-inspection of
  edge cases).
- Falls back gracefully on files it can't even parse as Python
  (rare but possible — broken `package.py` files exist in the wild).

## See also

- [Wiring pyrer into rez](../../getting-started/rez-integration/#lazy-package-discovery-on-cold-caches) — the `load_family` callback this parser composes with.
- [Issue #86](https://github.com/doubleailes/rer/issues/86) — the
  lazy-discovery hook that motivated this RFC.
- [`rustpython-parser` on crates.io](https://crates.io/crates/rustpython-parser)
  — the AST library this RFC depends on.
