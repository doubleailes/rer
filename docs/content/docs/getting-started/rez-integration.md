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

def load_family(name):
    """Return every PackageData for `name`, or [] if no such family."""
    pkgs = []
    for pkg in iter_packages(name, paths=PACKAGE_PATHS):
        pkgs.append(pyrer.PackageData.from_rez(pkg))
    return pkgs

result = pyrer.solve(
    ["maya-2024", "nuke-14"],
    packages=None,                # or a small eager seed
    load_family=load_family,
)
```

Semantics:

- The callback is called **at most once per family** in one solve
  (results are cached internally), and **only for families the
  solver actually exercises**.
- Returning `[]` means "no such family" — treated the same as a
  family that was never added.
- The `packages` argument is still accepted; entries supplied that
  way are pre-seeded into the cache and the callback is never asked
  for those families. Useful for a hybrid where you pre-load
  hot families and lazy-load the long tail.
- If the callback raises, the solve returns
  `result.status == "error"` with the exception message in
  `result.failure_description`. No exception escapes `pyrer.solve`.
- Defensive: entries whose `name` does not match the requested
  family are dropped; a duplicate `(family, version)` from the
  callback surfaces as `status="error"`.

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
