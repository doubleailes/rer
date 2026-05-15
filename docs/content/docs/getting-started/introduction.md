+++
title = "Introduction"
description = "What rer is, what it does, and where it sits in a rez workflow."
date = 2026-05-15T08:00:00+00:00
updated = 2026-05-15T08:00:00+00:00
draft = false
weight = 10
sort_by = "weight"
template = "docs/page.html"

[extra]
lead = "A faithful Rust port of rez's phase-based package solver, callable from Python via PyO3."
toc = true
top = false
+++

## What rer is

**r.e.r** stands for *"Rez En Rust"* — a pun on the Parisian suburban train
([réseau express régional d'Île-de-France](https://en.wikipedia.org/wiki/R%C3%A9seau_Express_R%C3%A9gional)).

`rer` is a Rust reimplementation of the **solver hotpath** of
[rez](https://github.com/AcademySoftwareFoundation/rez), the VFX / animation
package manager. The end goal is a **hybrid integration**: a Rust library
callable from Python via PyO3 that accelerates rez resolves while leaving
the rest of rez untouched.

## What "faithful port" means

The solver in `rer-resolver`'s `rez_solver` module is a port of
[`rez/src/rez/solver.py`](https://github.com/AcademySoftwareFoundation/rez/blob/main/src/rez/solver.py)
— not a different algorithm dressed up to look similar. It reproduces
rez's mechanics exactly:

- weak (`~`) and conflict (`!`) requirement semantics
- variant selection order
- the extract / intersect / reduce / split cycle
- implicit backtracking
- the same phase stack the Python solver uses

The goal is output that matches rez **1:1**, not merely "a valid resolve".
The earlier pubgrub-based solver was removed once the port matched rez
bit-for-bit on rez's bundled 188-case benchmark.

## Status

- ✅ **Solver** — complete and rez-faithful.
- ✅ **Validated 1:1** against rez's bundled 188-case benchmark dataset —
  every resolve matches rez's own recorded result (same status, same
  package set).
- ✅ **Fast** — on that benchmark, on one machine, `rer` resolves all 188
  requests in ~44s versus ~206s for rez 3.3.0 (`rez benchmark`).
  Correctness is version-independent; the timing is same-machine context,
  not a lab claim.
- ✅ **Python bridge** — `pip install pyrer`, then `pyrer.solve(...)`
  runs the ported solver.

## How rer fits into a rez workflow

rer works on an **in-memory package repository**: a mapping from
`family → version → {requires, variants}`. It does not read the
filesystem itself — the host (rez) hands the loaded data in.

That means rer is a drop-in accelerator for rez's resolve step, not a
replacement for rez. rez still discovers packages, parses `package.py`,
builds the environment, and manages contexts. rer just does the solving.

## Next steps

- **[Quick Start →](../quick-start/)** — install rer and run your first
  resolve from Python or Rust.
- **[Engineering notes →](../../engineering/)** — design decisions and
  measurements behind the port (e.g. why some rez optimisations are
  intentionally absent).
- **[Contributing →](../../contributing/how-to-contribute/)** — how to
  help.
- **[FAQ →](../../help/faq/)** — common questions.
