#!/usr/bin/env python3
"""Micro-benchmark the `pyrer.PackageData` construction paths.

Establishes the baseline for issue #88's perf claim: how much does
`from_rez(pkg)` actually cost vs. `from_strings(...)` / the four-arg
constructor, and how does that scale with package count? Run on
demand — not part of CI; results are machine-dependent.

Measures:
  1. Per-call construction cost for each path (μs, isolated).
  2. Per-batch construction cost for N packages (ms).
  3. End-to-end `pyrer.solve(...)` time with packages built each way.
  4. Construction-vs-solve share of total wall time.

Usage:
  python scripts/bench_python_construction.py
  python scripts/bench_python_construction.py --packages 500 --iters 200

Requires:
  pip install maturin
  cd crates/rer-python && maturin develop
"""
import argparse
import sys
import timeit
from typing import List

import pyrer


# ---------------------------------------------------------------------------
# Fake-rez Package mimics
# ---------------------------------------------------------------------------


class FakeRequirement:
    """Mimics `rez.version.Requirement` — not a str, only renders via __str__."""

    __slots__ = ("_s",)

    def __init__(self, s: str) -> None:
        self._s = s

    def __str__(self) -> str:
        return self._s


class FakeVersion:
    """Mimics `rez.version.Version` — not a str, only renders via __str__."""

    __slots__ = ("_s",)

    def __init__(self, s: str) -> None:
        self._s = s

    def __str__(self) -> str:
        return self._s


class FakeRezPackage:
    """Stand-in for `rez.packages.Package`. The four duck-typed attributes
    surface `FakeVersion` / `FakeRequirement` objects (not `str`) so that
    `from_rez` pays the `__str__` round-trip on each one — the cost issue
    #88 was filed against.
    """

    __slots__ = ("name", "version", "requires", "variants")

    def __init__(
        self,
        name: str,
        version: str,
        requires: List[str],
        variants: List[List[str]],
    ) -> None:
        self.name = name
        self.version = FakeVersion(version)
        self.requires = [FakeRequirement(r) for r in requires] if requires else None
        self.variants = (
            [[FakeRequirement(r) for r in v] for v in variants] if variants else None
        )


# ---------------------------------------------------------------------------
# Synthetic repo
# ---------------------------------------------------------------------------


def synth_packages(n: int):
    """Return three parallel lists for N packages:

    - raw_specs: (name, version, requires, variants) tuples — what
      `pkg.resource.data` would hand you (and what `from_strings`
      consumes).
    - fake_pkgs: `FakeRezPackage` instances — what `from_rez` consumes.
    - solver_inputs: the resolve seed (just "app" — any subset works).

    Each package has 3 requires and 2 variants of 2 entries each — a
    realistic shape that exercises the full attribute walk on `from_rez`.
    """
    raw_specs = []
    fake_pkgs = []
    for i in range(n):
        if i == 0:
            name, version = "app", "1.0.0"
            requires = ["lib", "util"]
            variants = []
        else:
            name = f"pkg{i:04d}"
            version = "1.0.0"
            requires = ["lib", "util"] if i < n // 2 else []
            variants = (
                [[f"python-3.{(i + 10) % 12}"], [f"python-3.{(i + 11) % 12}"]]
                if i % 3 == 0
                else []
            )
        raw_specs.append((name, version, requires, variants))
        fake_pkgs.append(FakeRezPackage(name, version, requires, variants))
    # lib / util — referenced by app's requires.
    for extra in (("lib", "1.0.0"), ("util", "1.0.0")):
        raw_specs.append((extra[0], extra[1], [], []))
        fake_pkgs.append(FakeRezPackage(extra[0], extra[1], [], []))
    return raw_specs, fake_pkgs


# ---------------------------------------------------------------------------
# Timers
# ---------------------------------------------------------------------------


def best_of(stmt, setup, number, repeat) -> float:
    """Median microseconds-per-call from a timeit.Timer run."""
    t = timeit.Timer(stmt=stmt, setup=setup, globals=globals())
    times = t.repeat(repeat=repeat, number=number)
    return (min(times) / number) * 1_000_000  # μs/call


def bench_single_call(raw_specs, fake_pkgs, iters: int) -> None:
    """Per-call construction cost for each path (a single random package)."""
    spec = raw_specs[0]
    fake = fake_pkgs[0]
    name, version, requires, variants = spec
    globals().update(
        spec=spec, fake=fake,
        name=name, version=version, requires=requires, variants=variants,
    )

    print("Per-call construction (median μs, one package, deep inputs)")
    print("-" * 60)

    t_new = best_of(
        "pyrer.PackageData(name, version, requires, variants)",
        "",
        number=iters,
        repeat=5,
    )
    t_strs = best_of(
        "pyrer.PackageData.from_strings(name, version, requires, variants)",
        "",
        number=iters,
        repeat=5,
    )
    t_rez = best_of(
        "pyrer.PackageData.from_rez(fake)",
        "",
        number=iters,
        repeat=5,
    )

    print(f"  PackageData(name, version, requires, variants):  {t_new:7.2f} μs")
    print(f"  PackageData.from_strings(...):                    {t_strs:7.2f} μs")
    print(f"  PackageData.from_rez(fake_rez_pkg):               {t_rez:7.2f} μs")
    delta_strs = t_rez - t_strs
    pct_strs = (delta_strs / t_rez) * 100
    print()
    print(f"  from_strings vs from_rez:  -{delta_strs:.2f} μs / -{pct_strs:.1f}%")
    delta_new = t_rez - t_new
    pct_new = (delta_new / t_rez) * 100
    print(f"  __new__       vs from_rez: -{delta_new:.2f} μs / -{pct_new:.1f}% "
          "(should match from_strings — same PyO3 path)")
    print()


def bench_batch(raw_specs, fake_pkgs, iters: int) -> None:
    """Total cost of materialising the whole repo N times."""
    globals().update(raw_specs=raw_specs, fake_pkgs=fake_pkgs)
    n = len(raw_specs)

    print(f"Per-batch construction (median ms, {n} packages built per iteration)")
    print("-" * 60)

    t_new = best_of(
        "[pyrer.PackageData(n, v, r, vr) for (n, v, r, vr) in raw_specs]",
        "",
        number=iters,
        repeat=5,
    )
    t_strs = best_of(
        "[pyrer.PackageData.from_strings(n, v, r, vr) for (n, v, r, vr) in raw_specs]",
        "",
        number=iters,
        repeat=5,
    )
    t_rez = best_of(
        "[pyrer.PackageData.from_rez(p) for p in fake_pkgs]",
        "",
        number=iters,
        repeat=5,
    )

    # best_of returns μs/call; one "call" is one batch of N constructions.
    print(f"  list-comp via PackageData(...):     {t_new / 1000:7.3f} ms / batch ({t_new / n:6.2f} μs/pkg)")
    print(f"  list-comp via from_strings(...):    {t_strs / 1000:7.3f} ms / batch ({t_strs / n:6.2f} μs/pkg)")
    print(f"  list-comp via from_rez(fake_pkg):   {t_rez / 1000:7.3f} ms / batch ({t_rez / n:6.2f} μs/pkg)")
    print()
    delta_ms = (t_rez - t_strs) / 1000
    print(f"  Savings switching from_rez → from_strings:  {delta_ms:6.3f} ms per batch")
    print()


def bench_end_to_end(raw_specs, fake_pkgs, iters: int) -> None:
    """Build + solve, isolating construction's share of total time."""
    globals().update(raw_specs=raw_specs, fake_pkgs=fake_pkgs)

    print("End-to-end pyrer.solve() — construction's share of wall time")
    print("-" * 60)

    t_solve_only = best_of(
        "pyrer.solve(['app'], pkgs)",
        "pkgs = [pyrer.PackageData.from_strings(n, v, r, vr) for (n, v, r, vr) in raw_specs]",
        number=iters,
        repeat=5,
    )
    t_e2e_strs = best_of(
        "pyrer.solve(['app'], [pyrer.PackageData.from_strings(n, v, r, vr) for (n, v, r, vr) in raw_specs])",
        "",
        number=iters,
        repeat=5,
    )
    t_e2e_rez = best_of(
        "pyrer.solve(['app'], [pyrer.PackageData.from_rez(p) for p in fake_pkgs])",
        "",
        number=iters,
        repeat=5,
    )

    print(f"  solve() alone (pre-built packages):       {t_solve_only / 1000:7.3f} ms")
    print(f"  build (from_strings) + solve:             {t_e2e_strs / 1000:7.3f} ms")
    print(f"  build (from_rez)     + solve:             {t_e2e_rez / 1000:7.3f} ms")
    print()
    build_share_strs = (t_e2e_strs - t_solve_only) / t_e2e_strs * 100
    build_share_rez = (t_e2e_rez - t_solve_only) / t_e2e_rez * 100
    print(f"  Build-phase share of total (from_strings):  {build_share_strs:5.1f}%")
    print(f"  Build-phase share of total (from_rez):      {build_share_rez:5.1f}%")
    print()
    e2e_delta = (t_e2e_rez - t_e2e_strs) / 1000
    e2e_pct = (t_e2e_rez - t_e2e_strs) / t_e2e_rez * 100
    print(f"  End-to-end savings switching to from_strings:  "
          f"{e2e_delta:.3f} ms ({e2e_pct:.1f}%)")
    print()


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--packages",
        type=int,
        default=50,
        help="Number of packages in the synthetic repo (default: 50)",
    )
    parser.add_argument(
        "--iters",
        type=int,
        default=500,
        help="`timeit` `number` argument — batches per repeat (default: 500)",
    )
    args = parser.parse_args()

    print(f"pyrer version: {getattr(pyrer, '__version__', '?')}")
    print(f"synthetic repo: {args.packages} packages")
    print(f"iterations per timing: {args.iters} (best of 5)")
    print()

    raw_specs, fake_pkgs = synth_packages(args.packages)

    bench_single_call(raw_specs, fake_pkgs, iters=args.iters * 10)
    bench_batch(raw_specs, fake_pkgs, iters=max(args.iters // 5, 50))
    bench_end_to_end(raw_specs, fake_pkgs, iters=max(args.iters // 10, 20))

    print("Note: these are Python-side construction costs. The Rust solver")
    print("itself is unchanged. End-to-end wall time on a real rez integration")
    print("will also include rez's own per-attribute AttributeForwardMeta cost")
    print("(not modelled by FakeRezPackage), so the from_rez line above is a")
    print("lower bound on the cost in production. The from_strings line is a")
    print("realistic upper bound for the fast path.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
