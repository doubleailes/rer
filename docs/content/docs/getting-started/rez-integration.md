+++
title = "Wiring `pyrer` into `rez`"
description = "How to use pyrer as the solver backend behind a normal rez workflow."
date = 2026-05-15T08:30:00+00:00
updated = 2026-05-15T08:30:00+00:00
draft = false
weight = 30
sort_by = "weight"
template = "docs/page.html"

[extra]
lead = "How to plug pyrer into a normal rez workflow: rez still handles package discovery and environment construction; pyrer just does the solve."
toc = true
top = false
+++

## What `pyrer` is, and what it is not

`pyrer` is **only the solver hotpath** — the rez-faithful phase-based
backtracking algorithm, ported to Rust and called from Python through
PyO3. It is *not* a replacement for `rez`. It does not:

- discover packages on the filesystem,
- parse `package.py` (it takes pre-parsed requirements as strings),
- build the runtime environment (PATH, env vars, shell hooks),
- handle the `rxt` context lifecycle, suites, or context bundling.

`rez` keeps doing all of that. `pyrer` is dropped in at the one step
where the cost lives: solving the version constraints.

The minimum integration looks like:

```
┌────────────────────────────────────────────┐
│ rez: iter_package_families / iter_packages │  ← package discovery
└────────────────────────────────────────────┘
                    │
                    ▼
┌────────────────────────────────────────────┐
│ build a pyrer repo dict (name → version    │
│ → {requires, variants})                    │  ← one-time conversion
└────────────────────────────────────────────┘
                    │
                    ▼
┌────────────────────────────────────────────┐
│ pyrer.solve(requests, packages)            │  ← the fast bit
└────────────────────────────────────────────┘
                    │
                    ▼
┌────────────────────────────────────────────┐
│ resolve → rez Variant objects →            │
│ ResolvedContext / env build                │  ← rez again
└────────────────────────────────────────────┘
```

## Building the `pyrer` repo from `rez`

`pyrer.solve()` accepts a Python list of `pyrer.PackageData` objects —
one per (package, version). Use `PackageData.from_rez(pkg)` to convert
each rez `Package` in one line:

```python
import pyrer
from rez.packages import iter_package_families


def build_pyrer_packages(package_paths):
    """Walk rez's package paths and yield pyrer.PackageData instances."""
    for family in iter_package_families(paths=package_paths):
        for pkg in family.iter_packages():
            yield pyrer.PackageData.from_rez(pkg)
```

`from_rez(pkg)` reads `name`, `version`, `requires` and `variants`
off the rez `Package`, stringifies each `Requirement` (rez's
`Requirement` instances are not `str` on their own — they render via
`__str__`), and stringifies `version` (a `rez.version.Version`). It
is duck-typed — `pyrer` itself does not import rez — so you can also
pass any object exposing the same four attributes (e.g. a test
fixture).

### Faster construction with `from_strings`

`from_rez(pkg)` triggers rez's `AttributeForwardMeta` chain on every
attribute and parses each requirement string into a `Requirement`
object only to immediately turn it back into a string. When you
already have the raw strings, prefer
`PackageData.from_strings(name, version, requires, variants)` —
it skips the wrapper round-trip entirely:

```python
def build_pyrer_packages_fast(package_paths):
    for family in iter_package_families(paths=package_paths):
        for pkg in family.iter_packages():
            data = pkg.resource.data
            # `data["requires"]` is a raw list[str] in the common
            # (non-late-bound) case; fall back to from_rez otherwise.
            if isinstance(data.get("requires", []), list) and \
               isinstance(data.get("variants", []), list):
                yield pyrer.PackageData.from_strings(
                    data["name"],
                    data["version"],
                    data.get("requires"),
                    data.get("variants"),
                )
            else:
                # @early / @late bindings — let rez evaluate them.
                yield pyrer.PackageData.from_rez(pkg)
```

The `from_strings` method:

- Skips the per-attribute `AttributeForwardMeta` lookup.
- Skips the `Requirement` parse (no `Version` / `VersionRange` AST
  is built then discarded).
- Skips the `str(Requirement)` round-trip per requirement.
- Accepts `None` for `requires` / `variants` (matches
  `dict.get(...)` ergonomics — no `or ()` boilerplate needed).

Functionally equivalent to the four-arg constructor; the
classmethod form exists so the contract has a name. **Always fall
back to `from_rez` for packages with `@early` or `@late` binding** —
in those cases `resource.data["requires"]` is a `SourceCode`
instance, not a `list[str]`, and `from_strings` will raise.

Two notes on this step:

- It is **eager** — every package on every path is loaded before the
  solve starts. `rez` normally loads lazily; the trade-off is one
  upfront cost vs many small ones during the solve. On a real repo
  on local disk with a warm page cache, eager loading is typically a
  few seconds; on the rez 188-case benchmark it is the dominant
  pre-solve cost.
- If you're running many resolves against the same repo in one
  process (CI, batch validation, a long-lived daemon), build the
  list **once** and reuse it.

If your repository sits on a slow filesystem (network mount, no
useful page cache), the eager load can easily exceed the solve
itself. The [next section](#lazy-package-discovery-on-cold-caches)
covers a callback-driven alternative that loads families on demand.

## Lazy package discovery on cold caches

`pyrer.solve` accepts an optional `load_family` callback that is
invoked the first time the solver needs a family it hasn't already
been given:

```python
import pyrer

def load_family(name, version_range=None):
    """Return every PackageData for `name` (optionally filtered to
    `version_range`), or [] if no such family.

    The `version_range` hint (issue #92) is a rez-syntax string —
    `"2+<3"`, `"==2.0"`, etc. — that the shim can pass directly to
    `iter_packages(range_=...)` so on-disk version dirs outside the
    range are skipped before any `package.py` is opened. `None`
    means "every version".
    """
    pkgs = []
    for pkg in iter_packages(name, range_=version_range, paths=PACKAGE_PATHS):
        pkgs.append(pyrer.PackageData.from_rez(pkg))
    return pkgs

result = pyrer.solve(
    ["maya-2024", "nuke-14"],
    packages=None,                # or a small eager seed
    load_family=load_family,
)
```

Semantics:

- The callback is called **at most once per (family, range)** in one
  solve (results are cached internally), and **only for families the
  solver actually exercises**.
- Returning `[]` means "no such family in the requested range" —
  treated the same as a family that was never added.
- The `packages` argument is still accepted; entries supplied that
  way are pre-seeded into the cache and the callback is never asked
  for those families. Useful for a hybrid where you pre-load
  hot families and lazy-load the long tail.
- **Backward-compatible signature** (issue #92): a callback defined
  as `def load_family(name):` keeps working — pyrer detects the
  signature and only passes `version_range` when the callback can
  accept it.
- If the callback raises, the solve returns
  `result.status == "error"` with the exception message in
  `result.failure_description`. No exception escapes `pyrer.solve`.
- Defensive: entries whose `name` does not match the requested
  family are dropped; a duplicate `(family, version)` from the
  callback surfaces as `status="error"`.

### The `version_range` hint (issue #92) — pre-filter at the shim

When a request pins a version (`maya-2024+`, `python<3.12`,
`==1.5.0`), pyrer propagates that constraint immediately. It then
passes the resulting range to the `load_family` callback as the
`version_range` keyword argument.

The shim can use it directly with rez's `iter_packages`:

```python
def load_family(name, version_range=None):
    return [
        pyrer.PackageData.from_rez(pkg)
        for pkg in iter_packages(name, range_=version_range, paths=PACKAGE_PATHS)
    ]
```

`rez.packages.iter_packages` already accepts a `range_` argument and
**skips entire version directories** that fall outside it, before
any `package.py` is opened. That cuts I/O proportionally to how
narrow the request is.

The hint is **advisory**:

- The shim **may** filter; pyrer re-validates anyway, so returning
  the full set is correct but wasteful.
- The shim **must not** drop versions outside the hint without
  reason — pyrer caches the loaded range and re-calls the loader
  with a widened range if a backtrack later needs more.
- `version_range=None` means "the solver needs every version" —
  always return the full family.

**Impact** on a 132-package resolve at a 5,500-package studio
(per the issue):

| Without hint | With hint |
|---:|---:|
| 2,637 packages loaded for 132 used (95% wasted) | ~400 packages loaded for 132 used (6.6× cut) |
| ~9 s `load_family` total | projected ~2 s |

The hint compounds with the static-`package.py` parser
([above](#plugging-in-the-static-package-py-fast-parser)) — the
parser makes each load cheap; the hint makes most loads
unnecessary.

### When this actually helps

The win is in I/O avoided, not in CPU. Specifically:

| Scenario | Lazy vs eager |
|---|---|
| Local disk, warm page cache, wide healthy resolve | Roughly equal — reachable ≈ touched, the eager cost is small anyway |
| Network filesystem (NFS / CIFS / SMB), studio-scale repo | **Substantial win** — every cold roundtrip avoided is a direct latency saving |
| Early-fail conflict resolves (e.g. `maya-2024 maya-2025`) | **Substantial win** — touches a handful of families instead of the whole reachable closure |
| Selective deep resolves in a large package universe | **Substantial win** — sparse subgraph means most reachable families are never opened |
| Single tool / CI probe inside a 5000-package store | **Substantial win** — same reason as above |

The shape of the win depends on the gap between the *reachable*
subgraph (eager BFS) and the *exercised* subgraph (what the solver
actually opens). When those diverge, lazy loading is essentially
free latency back.

### Worked example: Windows + CIFS

A common case: the rez repository lives on a Samba / CIFS share,
mounted on Windows clients. Windows has no equivalent of Linux's
page cache for SMB content, so every `rez env` invocation pays the
full network roundtrip for every `package.py` it opens — there is
no cross-invocation caching to amortise it. On that combination, the
eager BFS in the basic shim can easily dominate the wall-clock cost
of `rez env`, even though the solve itself runs in tens of
milliseconds.

`load_family` is the right primitive for this case: the solver only
asks the network for families it genuinely needs to inspect, and
each one is fetched at most once per resolve.

### Lazy variant of the shim

The monkey-patch shim becomes slightly simpler with the callback
form — no upfront BFS:

```python
import pyrer
import rez.solver as _rez_solver
import rez.resolver as _rez_resolver
from rez.packages import iter_packages
from rez.config import config as _rez_config

_original_resolve = _rez_resolver.Resolver._solve


def _pyrer_resolve(self):
    if self.package_filter or self.package_orderers:
        return _original_resolve(self)

    # Closure over the resolver's package paths — pyrer calls this
    # only for families the solver actually needs. The two-tier load:
    # try the Rust AST fast-parser first (skips Python evaluation for
    # the ~93% of `package.py` files at Fortiche that are static),
    # fall back to rez's `Package` evaluator for the dynamic ones
    # (`@early` / `@late` requires, top-level `if`, imports, …).
    def load_family(name):
        out = []
        for pkg in iter_packages(name, paths=self.package_paths):
            pd = None
            try:
                with open(pkg.filepath, "r", encoding="utf-8") as f:
                    source = f.read()
                pd = pyrer.parse_static_package_py(source)
            except OSError:
                pass
            if pd is None:
                # Dynamic file — `@early` / `@late` / imports / etc.
                pd = pyrer.PackageData.from_rez(pkg)
            out.append(pd)
        return out

    requests = [str(r) for r in self.package_requests]
    result = pyrer.solve(
        requests,
        packages=None,
        load_family=load_family,
        variant_select_mode=_rez_config.variant_select_mode,
    )

    if result.status != "solved":
        return _original_resolve(self)

    self.resolved_packages_ = resolve_to_rez_variants(
        result, self.package_paths,
    )
    self.status_ = _rez_solver.SolverStatus.solved
    return self


_rez_resolver.Resolver._solve = _pyrer_resolve
```

If the studio's `package_filter` configuration matters, apply it
inside `load_family` before returning the list — the filter then
runs only on families the solver actually exercises, instead of
every reachable family.

### What lazy loading does *not* fix

- **Cross-invocation cost.** `load_family` caches inside one solve;
  the next `rez env` invocation pays the load cost again for every
  family it touches. Closing that gap would need a persistent cache
  in the shim itself (keyed e.g. by `package.py` mtime). That sits
  outside `pyrer` — but `load_family` is the prerequisite that
  makes such a cache implementable as a wrapper around the
  callback.
- **GIL contention during the solve.** `pyrer.solve` currently
  holds the GIL for the duration of the resolve. In practice this
  rarely matters: the callback itself, when it does I/O via rez's
  loaders, releases the GIL inside the underlying C call. Other
  Python threads block only during the pure-Rust portions, which
  are short.
- **Solve-phase CPU.** The solver itself runs the same algorithm
  either way. Lazy loading is purely about avoiding pre-solve I/O.

## Plugging in the static `package.py` fast-parser

Once `load_family` is wired, the inner per-package load is the next
hot path. `pyrer.parse_static_package_py(source)` reads the four
solver-relevant fields directly from a `package.py` source string —
no Python interpreter, no `Requirement` parse, no `__str__`
round-trip. About **34.8× faster than `DeveloperPackage.from_path +
from_rez`** on the Fortiche-on-CIFS corpus
(75 μs/file vs 2.6 ms/file). Saves roughly 2.5 ms per file loaded —
~127 ms / resolve on a typical 50-family resolve.

The parser accepts the static subset of `package.py`: literal
assignments for `name`, `version`, `requires`, `variants`, plus
ignorable `def commands()` and `with scope("config")` blocks.
Anything dynamic (`@early` / `@late`, top-level `if`, `import`,
…) returns `None` so the caller falls back to rez. **Bias hard
toward bailing** — a false positive would diverge from rez and
produce a silent correctness regression in any resolve. See the
[engineering note](../../engineering/fast-package-py-parser/) for
the design, the corpus survey, the V2 hand-rolled-lexer rewrite,
and the differential test result (0 mismatches on 5,979
V2-accepted files at Fortiche).

### Two-tier `load_family`

The integration replaces one function in the shim:

```python
import os
import pyrer
from rez.packages import iter_packages


def _try_fast_parse(pkg):
    """Try the Rust static parser. Returns a PackageData on success,
    or None for any reason — non-.py package, unreadable file, or
    dynamic content the parser bails on. The caller falls back to
    `from_rez(pkg)`."""
    filepath = getattr(pkg, "filepath", None)
    if not filepath or not filepath.endswith(".py"):
        return None
    try:
        with open(filepath, "r", encoding="utf-8") as f:
            source = f.read()
    except OSError:
        return None
    return pyrer.parse_static_package_py(source)


def load_family(name, package_paths):
    out = []
    for pkg in iter_packages(name, paths=package_paths):
        pd = _try_fast_parse(pkg) or pyrer.PackageData.from_rez(pkg)
        out.append(pd)
    return out
```

That's the minimal integration. ~30 lines added; the existing
`_pyrer_resolve` method is unchanged except for using the
two-tier `load_family` above.

### Faster: batched parallel parse (issue #94)

The shape above is already a big win versus rez's `from_rez`, but
the inner Python loop is still serial — one `open()` per file,
per package, while the other cores idle. On a 2,600-file resolve
that's roughly 3 s of pure I/O even with the static parser doing its
job; `cProfile` shows it as the top of the flamegraph (35% of total
wall time) after every other pyrer win was wired up.

`pyrer.parse_static_packages_py(paths)` replaces that loop with a
single Rust call. It reads and parses every path on a Rayon thread
pool, returns a list aligned with the input, and releases the GIL
for the duration.

#### What it does

```python
pyrer.parse_static_packages_py(paths: list[str | os.PathLike])
    -> list[PackageData | None]
```

Per-path semantics are identical to `parse_static_package_py(source)`
— so the static-vs-dynamic decision, the corpus accept rate, and the
output `PackageData` shape are all unchanged. What's different is
that you pay one round-trip into Rust for the whole batch instead
of one per file, and the file reads + parses run in parallel.

- **Positionally aligned output.** `len(result) == len(paths)`. A
  missing file (`ENOENT`), unreadable bytes (permissions, invalid
  UTF-8), a parser bail on dynamic content, and a `package.yaml` /
  non-`.py` file all become `None` at the matching index. Callers
  can `zip(paths, result)` after.
- **No exception escapes.** Per-file failures map to `None`. The
  function as a whole only raises if the input type is wrong (e.g.
  passing a non-iterable for `paths`).
- **GIL released.** `Python::allow_threads` is used internally, so
  other Python threads run during the batch. The result list is
  built back on the GIL after the parallel section completes.
- **Pool size = `RAYON_NUM_THREADS`** (default: logical core count).
  No per-call knob. Cap with the env var on shared CI hosts.
- **Order doesn't depend on completion order.** Rayon's `par_iter`
  may finish files out of order; the returned list is rebuilt by
  index, so the shim's `zip(pkgs, result)` is always correct.

#### Measured impact on Fortiche

`scripts/bench_batched_parser.py` samples files from a real rez
repo, runs both paths best-of-3, and reports the speedup. On
`/thierry/rez/pkg` over CIFS:

| Sample | Serial `open` + parse | Batched | Speedup |
|---:|---:|---:|---:|
| 500 files (warm cache) | 56.71 ms | 40.76 ms | **1.39×** |
| 2,000 files | 4,234 ms | 1,508 ms | **2.81×** |

Per-file saving on the 2,000-file run: **~1.36 ms**. The issue's
target workload (132-package resolve, ~2,600 `package.py` files
touched) extrapolates to ~3.5 s saved per resolve.

The smaller-sample bench is bottlenecked on warm-page-cache parsing
CPU and the Rayon dispatch overhead amortises less. Real production
loads (cold or partially-cold CIFS, many uncached versions) see more
of the parallel-I/O overlap — the 2.81× is a lower bound on warm
hardware.

#### Integration: two-tier `load_family` with batched read

```python
import logging
import os
import pyrer
from rez.packages import iter_packages

_logger = logging.getLogger("pyrer.batched_parser")


def _gather_paths(pkgs):
    """Split rez Packages into (pkg, filepath_or_None). `None` means
    the package isn't filesystem-`.py`-based — yaml, memory repo,
    @early-bound — and goes straight to the slow rez path."""
    out = []
    for pkg in pkgs:
        filepath = getattr(pkg, "filepath", None)
        if filepath and filepath.endswith(".py"):
            out.append((pkg, filepath))
        else:
            out.append((pkg, None))
    return out


def load_family(name, package_paths, version_range=None):
    """The shim's load_family callback — batched fast-path + rez
    fallback per file. Wires together everything pyrer offers:
    #86 (load_family), #92 (version_range hint), the static parser,
    and #94 (batched parallel parse)."""
    # Apply the #92 hint at the iter level so rez skips on-disk
    # version dirs outside the range.
    pkgs = list(iter_packages(name, range_=version_range, paths=package_paths))
    pairs = _gather_paths(pkgs)
    paths = [fp for _, fp in pairs if fp is not None]

    # One Rust call across all .py files. GIL released; cores in use.
    pds = pyrer.parse_static_packages_py(paths) if paths else []
    pds_iter = iter(pds)

    out = []
    for pkg, filepath in pairs:
        if filepath is None:
            # Non-.py — straight to rez evaluator.
            out.append(pyrer.PackageData.from_rez(pkg))
            continue
        pd = next(pds_iter)
        if pd is None:
            # Parser bailed (dynamic content) or I/O error.
            out.append(pyrer.PackageData.from_rez(pkg))
        else:
            out.append(pd)
    return out
```

#### Backward compatibility / feature detection

The function is a pure addition. Shims that haven't been updated keep
working with the per-file `parse_static_package_py`. Feature-detect
once at shim init:

```python
_BATCHED_PARSE = hasattr(pyrer, "parse_static_packages_py")


def load_family(name, package_paths, version_range=None):
    if _BATCHED_PARSE:
        return _batched_load_family(name, package_paths, version_range)
    else:
        return _serial_load_family(name, package_paths, version_range)
```

This is the same pattern the shim uses for `load_family` (#86),
`version_range` (#92), and `parse_static_package_py` itself. No
flag day; pyrer < 1.0.0-rc.3 falls back automatically.

#### Shadow-validation mode

Same shape as the single-file parser: gate on an env var, run a
release with it on, log any divergence, then flip off.

```python
_VALIDATE = os.environ.get("REZ_PYRER_VALIDATE_BATCHED") == "1"


def _validate(pkg, batched_pd):
    """Compare `batched_pd` (from parse_static_packages_py) against
    what `from_rez(pkg)` would have produced. Used in production
    spot-checks during the rollout."""
    if not _VALIDATE or batched_pd is None:
        return
    rez_pd = pyrer.PackageData.from_rez(pkg)
    fast = (
        batched_pd.name, batched_pd.version,
        list(batched_pd.requires),
        [list(v) for v in batched_pd.variants],
    )
    slow = (
        rez_pd.name, rez_pd.version,
        list(rez_pd.requires),
        [list(v) for v in rez_pd.variants],
    )
    if fast != slow:
        _logger.warning(
            "pyrer batched parser DIVERGED for %s\n"
            "  fast: %r\n  slow: %r",
            getattr(pkg, "filepath", "?"), fast, slow,
        )
```

Spot-check the first hits each `load_family` call rather than
every package (the offline differential covered the full corpus —
this is a runtime sanity net). Drop the flag after a clean release.

#### Metrics — confirm what's actually happening

```python
class _PyrerStats:
    batched_calls = 0          # number of parse_static_packages_py invocations
    batched_files = 0          # sum of len(paths) across those calls
    batched_hits = 0           # files the batched call accepted
    batched_misses_io = 0      # missing / unreadable files
    batched_misses_dynamic = 0 # parser bailed
    non_py_packages = 0        # filesystem package isn't .py (yaml, memory repo)

    @classmethod
    def log_summary(cls):
        if not cls.batched_calls:
            return
        _logger.info(
            "pyrer batched parser: %d calls, %d files; "
            "hits=%d misses_io=%d misses_dynamic=%d non_py=%d",
            cls.batched_calls, cls.batched_files,
            cls.batched_hits, cls.batched_misses_io,
            cls.batched_misses_dynamic, cls.non_py_packages,
        )
```

Expected at Fortiche from the corpus survey: ~93% hit rate on
filesystem `.py` packages, with the misses split between
`@early`/`@late` dynamic packages and the occasional unreadable file.

#### Rollout plan

Three layered flags on top of the existing `use_rer_solver`:

```python
USE_RER_SOLVER = _rez_config.use_rer_solver
USE_BATCHED_PARSER = USE_RER_SOLVER and \
    hasattr(pyrer, "parse_static_packages_py") and \
    os.environ.get("REZ_PYRER_BATCHED_PARSER", "1") == "1"
VALIDATE_BATCHED_PARSER = USE_BATCHED_PARSER and \
    os.environ.get("REZ_PYRER_VALIDATE_BATCHED") == "1"
```

| Week | Flags | What to verify |
|---|---|---|
| 1 | `USE_BATCHED_PARSER=1`, `VALIDATE=1`, ~5% of users | 0 divergences in logs? Hit rate ≥ 90%? `_load_family` wall-time drops vs baseline? |
| 2 | Same flags, ~50% of users | Same, at scale. Confirm `RAYON_NUM_THREADS` not oversubscribing on shared hosts (check CPU%). |
| 3 | `USE_BATCHED_PARSER=1`, `VALIDATE=0`, 100% | Production wall-time + telemetry |
| 4 | Permanent in `use_rer_solver` config | Drop the env var |

Each step is one env var flip away from the previous behaviour.

#### Where this WON'T help

- **Resolves with very few packages touched.** The Rayon dispatch
  overhead is small (~µs per batch) but on a 5-package resolve
  there's almost nothing to parallelise. The win scales with batch
  size; tiny batches see negligible speedup.
- **The dynamic 7%.** `@early` / `@late`-bound packages still need
  rez's evaluator — those bypass the batched path entirely (see
  `non_py` / parser-bail accounting in the metrics).
- **`load_family` cache hits within a single `solve()`.** When a
  family is already cached on the pyrer side (#86), the loader
  isn't called — batched or otherwise.
- **Cross-invocation cost.** Each `rez env` is a fresh process and
  pays a fresh batched-parse cost. A persistent memcache of parsed
  `PackageData` is the next layer; this work is the prerequisite.

### Shadow-validation mode for the first weeks in production

The differential harness already ran clean on the Fortiche corpus
(5,813/5,813 matched). But a shadow check in production catches
runtime patterns the offline survey didn't exercise. Gate it on an
env var so it can be turned on for a release and off afterwards:

```python
import logging
import os

_VALIDATE = os.environ.get("REZ_PYRER_VALIDATE_PARSER") == "1"
_logger = logging.getLogger("pyrer.fast_parser")


def _try_fast_parse(pkg):
    filepath = getattr(pkg, "filepath", None)
    if not filepath or not filepath.endswith(".py"):
        return None
    try:
        with open(filepath, "r", encoding="utf-8") as f:
            source = f.read()
    except OSError:
        return None

    pd_fast = pyrer.parse_static_package_py(source)
    if pd_fast is None:
        return None

    if _VALIDATE:
        pd_slow = pyrer.PackageData.from_rez(pkg)
        fast_t = (
            pd_fast.name, pd_fast.version,
            list(pd_fast.requires),
            [list(v) for v in pd_fast.variants],
        )
        slow_t = (
            pd_slow.name, pd_slow.version,
            list(pd_slow.requires),
            [list(v) for v in pd_slow.variants],
        )
        if fast_t != slow_t:
            _logger.warning(
                "pyrer fast parser DIVERGED for %s\n"
                "  fast: %r\n  slow: %r\n"
                "  Using slow path; please report this upstream.",
                filepath, fast_t, slow_t,
            )
            return None  # divergence — fall back to rez
    return pd_fast
```

Run a release with `REZ_PYRER_VALIDATE_PARSER=1` set in the shim's
environment. Grep your studio logs for the warning; the count is
your real-world divergence rate. If it stays zero (which the
offline differential predicts), drop the flag.

### Metrics — confirm the hit rate

Class-level counters tell you what fraction of packages are
actually taking the fast path in production:

```python
class _PyrerStats:
    fast_hits = 0
    fast_misses_non_py = 0   # yaml package, memory repo, etc.
    fast_misses_dynamic = 0  # parser bailed (real dynamic content)
    fast_misses_io = 0       # couldn't read the file

    @classmethod
    def log_summary(cls):
        total = (cls.fast_hits + cls.fast_misses_non_py
                 + cls.fast_misses_dynamic + cls.fast_misses_io)
        if not total:
            return
        _logger.info(
            "pyrer fast parser: %d/%d hit (%.1f%%); "
            "non-py=%d dynamic=%d io=%d",
            cls.fast_hits, total, cls.fast_hits / total * 100,
            cls.fast_misses_non_py, cls.fast_misses_dynamic,
            cls.fast_misses_io,
        )
```

Increment from inside `_try_fast_parse`; call `log_summary` from
the shim's existing telemetry hook. Expected hit rate at Fortiche:
~93% based on the corpus survey.

### Rollout plan

Three flags stacked on top of the existing `use_rer_solver`:

```python
USE_RER_SOLVER = _rez_config.use_rer_solver
USE_FAST_PARSER = USE_RER_SOLVER and \
    os.environ.get("REZ_PYRER_FAST_PARSER", "1") == "1"
VALIDATE_FAST_PARSER = USE_FAST_PARSER and \
    os.environ.get("REZ_PYRER_VALIDATE_PARSER") == "1"
```

A reasonable rollout sequence:

| Week | Flags | What you're checking |
|---|---|---|
| 1 | `USE_FAST_PARSER=1`, `VALIDATE=1`, ~5% of users | Production divergence count = 0? Hit rate ≥ 90%? Wall-time A/B looks right? |
| 2 | Same flags, ~50% of users | Same checks at scale + memcache impact |
| 3 | `USE_FAST_PARSER=1`, `VALIDATE=0`, 100% | Production wall-time + telemetry |
| 4 | Flag stays on by default in `use_rer_solver` config | Make permanent |

Each step is a kill-switch flip away from the previous behaviour.

### Where this WON'T help (so nobody's surprised)

- **Cold `rez env` startup time.** The dominant ~200-300 ms of
  Python interpreter init is unchanged. The parser saves on the
  per-package-load step that happens *after* startup.
- **`@early` / `@late` packages.** These fall back to rez
  (the dynamic 7% at Fortiche). Studios with heavier late-bound use
  see proportionally less of the win.
- **`package.yaml`.** The parser is `.py` only; YAML goes through
  rez.
- **Resolves served from a cached `rxt` context.** rxt loading
  is its own path — the parser isn't invoked.
- **First invocation on a cold CIFS cache.** I/O dominates the
  file read; the parser saves CPU, not network roundtrips.

## Solving

```python
import pyrer

packages = list(build_pyrer_packages(["/sw/pkg", "/sw/site"]))

result = pyrer.solve(["maya-2024", "nuke-14"], packages)

print(result.status)        # "solved" | "failed" | "error"
print(result.solve_time_ms) # wall-clock of just the Rust solve
for variant in result.resolved_packages:
    print(variant.name, variant.version, variant.variant_index)
    print(variant.uri)      # "maya/2024.0/package.py[1]"
    print(variant.requires) # merged base + variant-specific requires
```

`status` distinguishes:

- **`"solved"`** — `result.resolved_packages` is a list of
  [`ResolvedVariant`] objects with `name`, `version`, `variant_index`,
  `requires`, and `uri`. `variant_index` is `None` for packages with no
  `variants` defined. The same resolution is also exposed as a list of
  `(name, version, variant_index)` tuples on `result.resolved` for
  callers that prefer that shape.
- **`"failed"`** — a real resolve conflict; `result.failure_description`
  has a human-readable reason.
- **`"error"`** — bad input (malformed repo, unparseable requirement
  string, missing top-level package).

No Python exception is raised from a failed or errored solve — both
are reported via `result.status`. Only a `TypeError` is raised, and
only when the `packages` argument is not a list of `PackageData`.

## Translating the result back to `rez`

`pyrer.ResolvedVariant` objects already expose the attribute surface
most rez consumers need (`name`, `version`, `variant_index`,
`requires`, `uri`). If you need rez's own `Variant` object (because
some downstream code reads attributes beyond that surface — built-in
`commands`, `private_build_requires`, `tools`, …), look it up from
rez:

```python
from rez.packages import get_package


def resolve_to_rez_variants(result, package_paths):
    """Turn pyrer.ResolvedVariant objects into rez Variants."""
    variants = []
    for rv in result.resolved_packages:
        pkg = get_package(rv.name, rv.version, paths=package_paths)
        if pkg is None:
            raise RuntimeError(f"package vanished after solve: {rv.name}-{rv.version}")
        # variant_index is None for packages with no variants — rez models
        # that as a single variant with index 0 internally.
        idx = rv.variant_index if rv.variant_index is not None else 0
        variants.append(pkg.get_variant(idx))
    return variants
```

These `Variant` objects can be fed into rez's normal context machinery
(see `rez.resolved_context.ResolvedContext` — you'll want to look at
how its internal solver result is normally consumed and substitute the
list above). For most workflows the most useful thing is to call
`rez.rex.bind` / `Variant.apply_value` on each variant against an
`ActionInterpreter`, which is the same code rez runs after its own
solve.

## A complete monkey-patch shim

If you want pyrer to transparently accelerate `rez env` /
`ResolvedContext` without changing call sites, the smallest sound
patch is to replace `rez.solver.Solver.solve` with a delegating
implementation. This is non-trivial to get right (rez's `Solver`
exposes a rich status surface — `phase_stack`, `failure_reason`,
graph rendering, callback support) so the patch is best kept narrow:
intercept the happy path, fall back to the real rez solver on any
non-default config (custom orderer, `late` binding requires,
`@early` evaluation, etc.).

The eager-loading shim below is the simplest form; for cold-cache
repos prefer the lazy variant shown
[earlier](#lazy-variant-of-the-shim), which lets the solver drive
the loading directly:

```python
import pyrer
import rez.solver as _rez_solver
import rez.resolver as _rez_resolver

_original_resolve = _rez_resolver.Resolver._solve


def _pyrer_resolve(self):
    # Fall back to rez on anything pyrer doesn't support yet.
    if self.package_filter or self.package_orderers:
        return _original_resolve(self)

    from rez.config import config as _rez_config

    packages = list(build_pyrer_packages(self.package_paths))
    requests = [str(r) for r in self.package_requests]
    result = pyrer.solve(
        requests,
        packages,
        variant_select_mode=_rez_config.variant_select_mode,
    )

    if result.status != "solved":
        return _original_resolve(self)  # let rez produce the canonical failure

    self.resolved_packages_ = resolve_to_rez_variants(
        result, self.package_paths,
    )
    self.status_ = _rez_solver.SolverStatus.solved
    return self


_rez_resolver.Resolver._solve = _pyrer_resolve
```

Load this once at process start (e.g. via a `rezconfig.py`'s
`plugin_path` entry or a `sitecustomize.py`) and any `rez env`,
`rez build`, `rez-bundle` etc. running in that process will route
through `pyrer` for the solve.

## Caveats and what isn't supported yet

`pyrer.solve` is the solver only. The following are **not** modelled
by it — if your studio depends on any of these, fall back to rez's
solver for those resolves:

- **`@early` / `@late` binding requires.** `pyrer` takes already-
  parsed strings; if a package's requires depend on the resolve
  context, rez has to evaluate them first.
- **Custom package orderers and filters.** Anything that hooks into
  `PackageOrder` / `PackageFilter` runs in rez; the integration shim
  above falls back when these are configured.
- **Cyclic-failure detail.** Both solvers detect cycles; the human-
  readable failure message differs in wording.

## Sanity-checking against rez

To make sure `pyrer` agrees with `rez`'s solver on your own repo,
generate a small set of representative requests and diff the
resolutions:

```python
from rez.resolved_context import ResolvedContext

packages = list(build_pyrer_packages(["/sw/pkg"]))

for request in your_real_requests:
    rer_result = pyrer.solve(request, packages)
    rez_ctx = ResolvedContext(request, package_paths=["/sw/pkg"])
    rer_set = {(v.name, v.version) for v in rer_result.resolved_packages}
    rez_set = {(v.name, str(v.version)) for v in rez_ctx.resolved_packages}
    assert rer_set == rez_set, f"diverge on {request}"
```

If any case diverges, open an issue with the request and a minimal
package set that reproduces — the project's correctness bar is "match
rez 1:1" and divergence is a release blocker.

## See also

- [Quick Start →](../quick-start/) — the basic `pyrer.solve` API in
  isolation, without any rez integration.
- [Engineering notes →](../../engineering/) — design decisions behind
  the port (e.g. why some rez optimisations are intentionally absent).
- [rez integration in the repo README](https://github.com/doubleailes/rer/blob/main/README.md)
  — short reference card.
