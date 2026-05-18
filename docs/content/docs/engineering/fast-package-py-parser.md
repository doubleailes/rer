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

> **Status: Stages 1–4 shipped.** Survey tool at
> `scripts/survey_package_py.py`; Rust parser crate at
> `crates/rer-package/`; PyO3 bindings (`pyrer.parse_static_package_py`
> and the batched `parse_static_packages_py`) at
> `crates/rer-python/src/lib.rs`; differential safety net at
> `scripts/diff_against_rez.py`; perf benches at
> `scripts/bench_package_py_parser.py` and
> `scripts/bench_batched_parser.py`. Stage 1 numbers, Stage 3
> per-file timing, Stage 2 differential (0 mismatches on 5,979
> files), and Stage 4 batched speedup (2.81× on 2,000 files) are
> all inline below.

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

Two iterations, both run with `scripts/bench_package_py_parser.py
--corpus /thierry/rez/pkg --samples 100 --iters 30 --with-rez`
against the live Fortiche-on-CIFS repo and rez 3.3.0.

### V1: AST-based parser (`rustpython-parser`)

| Path | μs / file | Speedup |
|---|---:|---:|
| `open + read + parse_static_package_py` | 1,533 | — |
| `DeveloperPackage.from_path + from_rez` (rez) | 2,632 | — |
| **Result** | | **1.7×** |

The parse step alone was 1,990 μs — `rustpython-parser` builds the
full module AST when we only need four top-level fields. It also
adds ~30 MB of crate deps to the build. Diagnosis: wrong tool.

### V2: hand-rolled lexer

A 700-line module-level scanner that walks the four patterns
(`name = "..."`, `version = "..."`, `requires = [...]`,
`variants = [[...]]`) directly, with bracket / string / comment /
indent tracking but no AST allocation. Same public API as V1.

| Path | μs / file | Speedup |
|---|---:|---:|
| `open + read + parse_static_package_py` | **75.24** | — |
| `DeveloperPackage.from_path + from_rez` (rez) | **2,615.54** | — |
| **Result** | | **34.8×** |

In-memory breakdown:

| Path | μs / file |
|---|---:|
| `parse_static_package_py(source)` (hand-rolled scan) | 59.23 |
| `from_rez(fake_pkg)` (attribute walk, lower bound) | 11.17 |
| `from_rez(real rez Package)` (post-load, just walk) | 13.50 |
| file read alone (open + read, warm cache) | 14.81 |

The parse step alone dropped from 1,990 μs (V1) to **59 μs (V2)** —
~33× on that layer, lifting the full-load comparison vs rez from
1.7× to **34.8×**. Per-file savings: ~2.54 ms. Over a 50-family
resolve, that's **~127 ms saved per resolve** — real, artist-
perceptible latency.

The V2 rewrite held 92.9% acceptance on the Fortiche corpus
(5,979 / 6,439 vs V1's 5,985; 6 files of drift, well within
rounding). Two non-obvious bug categories surfaced during the
rewrite, both Windows-specific:

- CRLF line endings — Samba-served `package.py` files end lines with
  `\r\n`. The scanner needed `\r` treated transparently in inline
  whitespace.
- `\<CRLF>` line continuations on non-solver assignments (e.g.
  `changelog = \` followed by a CRLF then a multi-line triple-
  quoted string). The continuation handler only recognised `\<LF>`.

Both have dedicated unit tests in the V2 implementation.

### What the bench numbers say about the next ceiling

V2's 75 μs/file full-load splits as:

- ~15 μs file I/O (warm-cache CIFS)
- ~60 μs parsing CPU

CPU is no longer dominant; I/O is. Further parser optimisations have
diminishing returns. The next 10× lives in either avoiding more I/O
(`load_family` from #86 already does most of this — files the solver
never asks for never get loaded) or a parsed-package cache layered
on top of Fortiche's existing shared memcache.

### Caveats on the 34.8× headline

- **Cold-cache CIFS** — the 14.81 μs file-read is a warm-cache
  number. On a truly cold network read, file I/O can be 1–100 ms,
  swamping the 60 μs parse cost. The Rust parser's relative
  advantage over rez stays the same proportionally (rez pays the
  same I/O), but the absolute saving per file shifts from 2.5 ms
  toward whatever the network roundtrip costs.
- **Sample bias** — 100 files sampled deterministically from the
  corpus. The full 6,439-file corpus could behave differently in
  pathological cases (very large files, unusual structure). The
  unit tests + the 6-file drift between V1 and V2 acceptance count
  is the existing safety net.
- **No production A/B yet** — the 127 ms / resolve number is from
  micro-bench arithmetic, not a real `rez env` measurement.
  Production wall time may show less if other phases dominate.

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| **Silent correctness regression** — the fast parser accepts a file it shouldn't | Bias hard toward bailing in `classify.rs`. Run the differential test on every file the fast parser claims. Treat any mismatch as a release blocker. |
| **Maintenance burden** — `rustpython-parser` is a Python AST library; track upstream Python syntax changes | Pin to a known-good version. Studio `package.py` files don't typically use bleeding-edge syntax. |
| **Coverage drift** — over time studios add patterns the parser doesn't handle | The fast path is opt-in. Coverage drift means the slow path runs more often, not that correctness breaks. We can extend coverage when patterns become common. |
| **Stage 1 says "don't build it"** | We've spent two days on a survey and now know the workload. Pivot to the memcache route below or accept the status quo. Cheap pivot point. |

## Stage 2 safety net — differential against rez

The bias-toward-bailing policy is only safe if "V2 accepts a file"
also means "V2 produces the same `(name, version, requires, variants)`
that rez does". A divergence here is a silent correctness regression
in any rez integration shim that uses the fast path.

`scripts/diff_against_rez.py` is the test harness: for every file V2
accepts, it also loads via rez's `DeveloperPackage.from_path` and
stringifies the four fields with `str(req)`, then compares
byte-for-byte. Any divergence is a release blocker, exactly like
the 188-case rez solver differential.

### Result on /thierry/rez/pkg + rez 3.3.0

| | Count | % of V2-accepted |
|---|---:|---:|
| Total files surveyed | 6,439 | — |
| V2 accepted | 5,979 | — |
| V2 bailed (slow path) | 460 | — |
| **Match (all four fields agree with rez)** | **5,813** | **97.22%** |
| **Mismatch (correctness regression)** | **0** | **0.00%** |
| rez evaluation error | 166 | 2.78% |

Wall-clock for the full run: 74 seconds (CIFS warm). Zero
mismatches over the entire Fortiche corpus — the safety net is
green.

### What about the 166 rez-eval-error files?

These are files V2 accepts but rez 3.3.0 in this dev venv can't
load. Sampled five of them — all five are the same error:

  InvalidPackageError: Package … uses @include decorator, but no
  include path has been configured with the 'package_definition_python_path'
  setting.

That's a **rez environment-config issue, not a content issue.**
Production rez at Fortiche has `package_definition_python_path`
set; rez would load these files fine. V2 correctly accepts them
because `@include def some_func():` is a non-solver decorator (the
function being decorated isn't `requires` / `variants`), and the
four solver fields are static.

So the "166 rez-eval-error" bucket is really "files this dev venv
can't evaluate because of a missing config knob" — not a
correctness signal. In production they'd match.

If the differential test ever needs to be tightened, the path is to
also configure `package_definition_python_path` in the dev venv —
but that's CI infrastructure, not a parser change.

## Stage 4 — Batched parallel parse (issue #94)

After Stages 1–3 landed, `cProfile` of a real Fortiche resolve
showed the static parser itself was no longer in the top of the
flamegraph. The cost had moved one layer up: the shim's serial
Python loop of `open()` calls feeding the parser. On a 132-package
resolve that was 3.20 s of pure I/O (35% of total wall time), one
file at a time while seven cores idled.

`parse_static_packages_py(paths)` is the response: open + parse
every path in one Rust call across a Rayon thread pool, with the
GIL released for the whole batch. Same per-file semantics as
`parse_static_package_py` — accept rate, output shape, differential
correctness all carry over.

### Result on Fortiche

`scripts/bench_batched_parser.py` against `/thierry/rez/pkg` over
CIFS, best-of-3:

| Sample | Serial `open` + parse | Batched | Speedup |
|---:|---:|---:|---:|
| 500 files (warm cache) | 56.71 ms | 40.76 ms | **1.39×** |
| 2,000 files | 4,234 ms | 1,508 ms | **2.81×** |

Per-file saving on the 2,000-file run: **~1.36 ms**. Extrapolated
to the issue's target workload (132-package resolve, ~2,600
`package.py` files): **~3.5 s saved per resolve**.

Both paths produce identical accepts (1,864/2,000 → static-parseable
fraction matches the per-file parser) — zero correctness drift.

The 500-file bench is bottlenecked on warm-page-cache parsing CPU;
the Rayon dispatch overhead amortises less on small batches. On
cold-cache or larger batches the parallel-I/O overlap shows
through. The 2.81× is a lower bound on warm hardware; on cold CIFS
(Windows production) it should grow.

### Design choices

- **Output is positionally aligned with input.** Missing files,
  unreadable bytes, and parser bails all become `None` at the
  matching index. The shim's `zip(pkgs, result)` is then trivially
  correct.
- **No exception escapes.** Per-file failures map to `None`. The
  function only raises if the input type is wrong.
- **Pool size = Rayon default** (`RAYON_NUM_THREADS` env var or
  logical core count). No per-call knob initially; capacity
  control is environmental.
- **Pure addition.** The single-file API stays. Shims feature-detect
  with `hasattr(pyrer, "parse_static_packages_py")` and fall back
  to the per-file loop on older pyrer.

### Safety net

Reused from Stage 2. The same `from_rez(pkg)` comparison can be
shadow-checked at production runtime, gated on
`REZ_PYRER_VALIDATE_BATCHED`. The integration page in the
[rez integration docs](../../getting-started/rez-integration/#shadow-validation-mode-1)
has the recipe.

The offline Stage 2 differential — 5,813 / 5,813 matched on the
Fortiche corpus — covers the per-file semantics. Stage 4's batched
call uses the exact same `parse_static_package_py` per file, so the
existing safety net carries over byte-for-byte; the only Stage 4
specific risk is around ordering / completion which the
positional-alignment contract handles explicitly.

### What's next after this

Stage 4 takes us to:

- ~93 % of `package.py` files served by the Rust fast path
- ~75 µs per file via the static parser (Stage 3)
- ~1/3 the wall-time on the open+parse phase via the batched call (Stage 4)

The remaining cost on `_load_family` is now real I/O (CIFS round-
trips for files Rayon's pool can't overlap further) plus the
dynamic-7 % rez evaluator path. Both are architectural —
addressing them needs a layer outside this RFC (memcache caching
of parsed `PackageData` across invocations is the obvious next
move, as called out in the "Considered alternatives" section
below).

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
