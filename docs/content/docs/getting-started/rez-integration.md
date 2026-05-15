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
│ pyrer.solve(requests, json.dumps(repo))    │  ← the fast bit
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
one per (package, version). Build them straight off rez's loaded
packages, no JSON serialisation needed:

```python
import pyrer
from rez.packages import iter_package_families


def build_pyrer_packages(package_paths):
    """Walk rez's package paths and yield pyrer.PackageData instances."""
    for family in iter_package_families(paths=package_paths):
        for pkg in family.iter_packages():
            yield pyrer.PackageData(
                name=family.name,
                version=str(pkg.version),
                requires=[str(r) for r in (pkg.requires or [])],
                variants=[
                    [str(r) for r in variant]
                    for variant in (pkg.variants or [])
                ],
            )
```

Two notes on this step:

- It is **eager** — every package on every path is loaded. `rez`
  normally loads lazily; the trade-off is one upfront cost vs many
  small ones during the solve. On a real repo, eager loading is
  typically a few seconds; on the rez 188-case benchmark it is the
  dominant pre-solve cost.
- If you're running many resolves against the same repo (CI, batch
  validation, a long-lived daemon), build the list **once** and reuse
  it.

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
only when the `packages` argument is not a list of `PackageData` (or
a JSON string — see below).

### Backward compatibility: JSON string

For callers that already serialise the repo as JSON, `pyrer.solve`
still accepts that shape — `{name: {version: {"requires": [...],
"variants": [[...]]}}}` rendered with `json.dumps`. The internals
deserialise it into the same `PackageData` list as the new form, so
the result is identical.

```python
import json
result = pyrer.solve(["app"], json.dumps({"app": {"1.0.0": {...}}}))
```

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

The minimum viable shim — for studios with default-configured repos —
looks roughly like:

```python
import pyrer
import rez.solver as _rez_solver
import rez.resolver as _rez_resolver

_original_resolve = _rez_resolver.Resolver._solve


def _pyrer_resolve(self):
    # Fall back to rez on anything pyrer doesn't support yet.
    if self.package_filter or self.package_orderers:
        return _original_resolve(self)

    packages = list(build_pyrer_packages(self.package_paths))
    requests = [str(r) for r in self.package_requests]
    result = pyrer.solve(requests, packages)

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

- **`VariantSelectMode::intersection_priority`.** `pyrer` implements
  rez's default `version_priority` only; see
  [issue #63](https://github.com/doubleailes/rer/issues/63).
- **`@early` / `@late` binding requires.** `pyrer` takes already-
  parsed strings; if a package's requires depend on the resolve
  context, rez has to evaluate them first.
- **Custom package orderers and filters.** Anything that hooks into
  `PackageOrder` / `PackageFilter` runs in rez; the integration shim
  above falls back when these are configured.
- **Cyclic-failure detail.** Both solvers detect cycles; the human-
  readable failure message differs in wording.
- **Variant-index parity.** The differential test currently checks
  the resolved `(name, version)` set, not the variant index — variant
  selection is rez-faithful by construction but is not enforced by
  the test suite. See the [README's *Validated 1:1* note](../../../../#status).

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
