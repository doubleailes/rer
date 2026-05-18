#!/usr/bin/env python3
"""Bisect tool for issue #96-style "pyrer diverges from rez" reports.

Resolves N requests through both engines (pyrer directly + rez's
`ResolvedContext`) and reports which agree, which diverge, and which
fail asymmetrically. Uses the recommended shim shape from the
rez-integration docs: `load_family` with `iter_packages(range_=...)`,
the static parser, and `from_rez` fallback.

If this script reports **0 divergences** on your corpus, the
recommended shim shape works correctly. Any divergence reported by a
downstream rez integration shim is then a shim-translation bug, not
a pyrer-side correctness bug. The fix lives in the shim.

If this script DOES reproduce a divergence, that's a pyrer bug —
please report with the failing request, the corpus path, and the
output of this script.

## Usage

  python scripts/compare_resolves.py /path/to/rez/repo
  python scripts/compare_resolves.py /path/to/repo --request foo bar
  python scripts/compare_resolves.py /path/to/repo --n 30 --seed 42

Requires `rez` and `pyrer` importable. The script doesn't touch
`config.use_rer_solver` — rez always uses its own Python solver,
pyrer is called directly via `pyrer.solve()`.
"""
import argparse
import os
import random
import sys

import pyrer
from rez.config import config
from rez.packages import iter_packages
from rez.resolved_context import ResolvedContext


def make_load_family(package_paths):
    """The recommended shim shape: static parser fast path + rez
    fallback on dynamic / unreadable packages."""

    def load_family(name, version_range=None):
        out = []
        for pkg in iter_packages(name, range_=version_range, paths=package_paths):
            filepath = getattr(pkg, "filepath", None)
            if filepath and filepath.endswith(".py"):
                try:
                    with open(filepath, "r", encoding="utf-8") as f:
                        source = f.read()
                    pd = pyrer.parse_static_package_py(source)
                except OSError:
                    pd = None
                if pd is None:
                    pd = pyrer.PackageData.from_rez(pkg)
            else:
                pd = pyrer.PackageData.from_rez(pkg)
            out.append(pd)
        return out

    return load_family


def family_names(package_paths):
    """Top-level family directories across all package paths."""
    families = []
    for prefix in package_paths:
        if not os.path.isdir(prefix):
            continue
        for d in os.listdir(prefix):
            if d.startswith(".") or d.startswith("_"):
                continue
            full = os.path.join(prefix, d)
            if os.path.isdir(full):
                families.append(d)
    return sorted(set(families))


def compare_one(request, package_paths, load_family):
    """Resolve `request` via both engines (no implicits) and
    classify the outcome."""
    rer = pyrer.solve(request, None, load_family=load_family)

    # rez side — same package_paths, no implicits.
    config.implicit_packages = []
    try:
        ctx = ResolvedContext(
            package_requests=request,
            package_paths=package_paths,
            add_implicit_packages=False,
        )
    except Exception as e:
        return ("rez_exception", rer, None, str(e))

    rer_ok = rer.status == "solved"
    rez_ok = bool(ctx.success)
    if rer_ok and rez_ok:
        rer_set = {(v.name, v.version, v.variant_index) for v in rer.resolved_packages}
        rez_set = {
            (v.name, str(v.version), getattr(v, "index", None))
            for v in (ctx.resolved_packages or [])
        }
        verdict = "identical" if rer_set == rez_set else "diverge"
        return (verdict, rer, ctx, None)
    if not rer_ok and not rez_ok:
        return ("both_fail", rer, ctx, None)
    return ("rer_fail" if not rer_ok else "rez_fail", rer, ctx, None)


def show_divergence(name, rer, rez):
    """Print package-by-package diff for a divergent resolve."""
    rer_set = {(v.name, v.version, v.variant_index) for v in rer.resolved_packages}
    rez_set = {
        (v.name, str(v.version), getattr(v, "index", None))
        for v in (rez.resolved_packages or [])
    }
    only_rer = rer_set - rez_set
    only_rez = rez_set - rer_set
    from collections import defaultdict
    by_name = defaultdict(dict)
    for n, v, i in only_rer:
        by_name[n]["rer"] = (v, i)
    for n, v, i in only_rez:
        by_name[n]["rez"] = (v, i)
    print(f"\n  --- {name}: {len(by_name)} packages differ ---")
    for n in sorted(by_name)[:30]:
        r = by_name[n].get("rer", ("—", "—"))
        z = by_name[n].get("rez", ("—", "—"))
        print(f"    {n:35s}  rer={r[0]}[{r[1]}]   rez={z[0]}[{z[1]}]")


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument(
        "package_paths",
        nargs="+",
        help="rez package paths (one or more)",
    )
    ap.add_argument(
        "--request",
        nargs="+",
        default=None,
        help="Specific package family names to resolve (overrides sampling)",
    )
    ap.add_argument(
        "--n",
        type=int,
        default=20,
        help="If --request not given, sample N random families (default: 20)",
    )
    ap.add_argument(
        "--seed",
        type=int,
        default=42,
        help="RNG seed for reproducible sampling (default: 42)",
    )
    args = ap.parse_args()

    for p in args.package_paths:
        if not os.path.isdir(p):
            print(f"not a directory: {p}", file=sys.stderr)
            return 2

    load_family = make_load_family(args.package_paths)

    if args.request:
        sample = args.request
        print(f"comparing {len(sample)} requested families")
    else:
        fams = family_names(args.package_paths)
        print(f"corpus: {len(fams)} top-level families")
        random.seed(args.seed)
        sample = random.sample(fams, min(args.n, len(fams)))
        print(f"sampling {len(sample)} families (seed={args.seed})")
    print()

    counts = {}
    diverge_samples = []
    rer_fail_samples = []

    for i, name in enumerate(sample, 1):
        verdict, rer, rez, err = compare_one([name], args.package_paths, load_family)
        counts[verdict] = counts.get(verdict, 0) + 1
        marker = {
            "identical": "✓",
            "diverge": "≠",
            "rer_fail": "x",
            "rez_fail": "X",
            "both_fail": "·",
            "rez_exception": "!",
        }.get(verdict, "?")
        print(f"  [{i:3d}/{len(sample)}] {marker} {name}")
        if verdict == "diverge":
            diverge_samples.append((name, rer, rez))
        elif verdict == "rer_fail":
            rer_fail_samples.append((name, rer))

    print()
    print("Summary:")
    for verdict in sorted(counts):
        print(f"  {verdict:15s}  {counts[verdict]}")
    total = sum(counts.values())
    diverge = counts.get("diverge", 0) + counts.get("rer_fail", 0)
    pct = (total - diverge) / max(total, 1) * 100
    print(f"\n  agreement: {pct:.1f}% ({total - diverge}/{total})")

    if diverge_samples:
        print()
        print("=== Divergent resolves ===")
        for name, rer, rez in diverge_samples[:3]:
            show_divergence(name, rer, rez)

    if rer_fail_samples:
        print()
        print("=== Rer-only failures ===")
        for name, rer in rer_fail_samples[:3]:
            print(f"  {name}: {rer.failure_description}")

    return 0 if not diverge_samples and not rer_fail_samples else 1


if __name__ == "__main__":
    sys.exit(main())
