+++
title = "FAQ"
description = "Answers to frequently asked questions about rer."
date = 2026-05-15T19:30:00+00:00
updated = 2026-05-15T19:30:00+00:00
draft = false
weight = 30
sort_by = "weight"
template = "docs/page.html"

[extra]
lead = "Answers to frequently asked questions about rer."
toc = true
top = false
+++

## What is rer?

`rer` ("Rez En Rust") is a Rust reimplementation of the solver hotpath
of [rez](https://github.com/AcademySoftwareFoundation/rez), the VFX
package manager. It is a **faithful port** of rez's own phase-based
backtracking solver (`rez/src/rez/solver.py`), not a different algorithm
— so resolves match rez 1:1 on the 188-case benchmark dataset rez ships.

## Is rer a replacement for rez?

No. rer is the **solver hotpath only**. rez still discovers packages,
parses `package.py`, builds the environment, and manages contexts. rer
just resolves: it takes an in-memory package repository plus a request
list and returns a resolution.

## How do I use rer from rez?

The intended integration is via the Python bridge: `pip install pyrer`,
then `import pyrer` and call `pyrer.solve(requests, packages_json)`. The
host (rez) loads packages as it already does and hands them to rer as
JSON. End-to-end rez integration is still being prototyped — track
progress on the [repo](https://github.com/doubleailes/rer).

## Why is the Python package called `pyrer` and not `rer`?

The name `rer` is already taken on PyPI. The wheel ships as `pyrer` and
the Python import is also `pyrer` (`pip install pyrer` → `import pyrer`).
The Rust crates on crates.io keep the `rer-version` / `rer-resolver`
names.

## Is rer faster than rez?

Yes — on the rez benchmark, on one machine, rer resolves all 188 cases
in **~44 seconds** versus **~206 seconds** for `rez benchmark` on
rez 3.3.0. Correctness is version-independent; the timing is
same-machine context, not a lab claim.

## What about variants, weak (`~`) and conflict (`!`) requirements?

All supported, with rez-faithful semantics. The solver is a port of
[`rez/src/rez/solver.py`](https://github.com/AcademySoftwareFoundation/rez/blob/main/src/rez/solver.py),
so variant selection order, the extract / intersect / reduce / split
phases, and implicit backtracking all behave as rez does.

## Where does rer get package data from?

From the caller. rer never reads the filesystem — there is no
`package.py` parser in Rust. The host (rez, or a test harness) loads
packages and passes them in as `pyrer.PackageData` instances:
`name -> version -> {requires, variants}`.

The host can pass them in two ways:

- **Eager** — a `list[PackageData]` built up front. Simple; best
  when the repo is on local disk or already cached in memory.
- **Lazy** — a `load_family(name) -> list[PackageData]` callback
  that the solver invokes only for families it actually touches.
  Better when the repo lives on a slow filesystem (network mounts,
  CIFS, NFS without a useful page cache) or when typical resolves
  exercise a small subgraph of a large package store. See the
  [rez integration page](../../getting-started/rez-integration/#lazy-package-discovery-on-cold-caches).

## Is there a CLI?

Not yet. There was a placeholder `rer` binary; it was removed because
it did nothing. A real CLI (build / bind / env subcommands) is future
work. For now, drive the solver from Python (`pyrer`) or from Rust
(`rer-resolver`).

## Which Python versions does `pyrer` support?

CPython 3.9+ on Linux, macOS, and Windows. The wheel is built against
the stable ABI (`abi3-py39`), so one wheel per platform/arch covers
every supported Python version.

## Where do I report a regression against rez?

Open an issue at
[github.com/doubleailes/rer/issues](https://github.com/doubleailes/rer/issues)
with the request list, a minimal package repository, and the diverging
resolution. See the [contributing guide](../../contributing/how-to-contribute/)
for the full template.

## Keyboard shortcuts for search

- focus: `/`
- select: `↓` and `↑`
- open: `Enter`
- close: `Esc`

## Other documentation

- [rez](https://rez.readthedocs.io/) — the upstream package manager.
- [Zola](https://www.getzola.org/documentation/getting-started/overview/) —
  the static site generator this documentation is built with.
