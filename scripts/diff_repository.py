#!/usr/bin/env python3
"""Differential test: rez vs rer on package *repository* loading.

For every family in the given ``packages_path``, compare

- rez: ``rez.packages.iter_packages(name, range_, paths)`` and each
  package's ``resource._data``;
- rer: ``pyrer.PackageRepository(paths).iter_packages(name, range_)``.

rer may hand a whole family (``NeedsPython`` with reason ``Family``) or a
single package back to rez. Otherwise the yielded ``(name, version)`` set
must be identical, and every package rer loads must have exactly rez's
data. Each family is checked unconstrained and with a few ranges built
from its own versions.

Usage::

    python scripts/diff_repository.py PATH [PATH ...] [--show N]

Exit status is 1 on any blocker.
"""
import argparse
import collections
import os
import random
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import pyrer  # noqa: E402
from rez.package_repository import package_repository_manager  # noqa: E402
from rez.packages import iter_package_families, iter_packages  # noqa: E402

from diff_definitions import first_difference, norm  # noqa: E402


def rez_family(name, paths, range_):
    try:
        pkgs = list(iter_packages(name, range_=range_, paths=paths))
    except Exception as e:
        return ("error", "%s: %s" % (type(e).__name__, e))
    out = {}
    for pkg in pkgs:
        try:
            data = norm(pkg.resource._data)
        except Exception as e:
            data = ("error", type(e).__name__)
        out[(pkg.name, str(pkg.version))] = data
    return ("ok", out)


def compare_family(repo, name, paths, range_, counts, problems):
    rer = repo.iter_packages(name, range_)
    rez = rez_family(name, paths, range_)
    label = "%s %s" % (name, range_ or "")
    if isinstance(rer, pyrer.NeedsPython):
        counts["family_needs_python"] += 1
        return
    if rez[0] == "error":
        counts["BLOCKER"] += 1
        problems.append((label, "rer listed the family, rez raised: " + rez[1]))
        return
    rez_pkgs = rez[1]
    rer_keys = {(n, v) for n, v, _, _ in rer}
    if rer_keys != set(rez_pkgs):
        counts["BLOCKER"] += 1
        problems.append((label, "package sets differ: only rez %s, only rer %s" % (
            sorted(set(rez_pkgs) - rer_keys), sorted(rer_keys - set(rez_pkgs)))))
        return
    counts["family_match"] += 1
    for n, v, _fp, data in rer:
        if isinstance(data, pyrer.NeedsPython):
            counts["package_needs_python"] += 1
            continue
        a, b = rez_pkgs[(n, v)], norm(data)
        if a == b:
            counts["package_match"] += 1
        else:
            counts["BLOCKER"] += 1
            problems.append(("%s-%s" % (n, v), first_difference(a, b) if a[0] != "error" else "rez raised " + a[1]))


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("paths", nargs="+")
    ap.add_argument("--show", type=int, default=20)
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--ranges", type=int, default=3, help="extra ranges checked per family")
    args = ap.parse_args()

    paths = [os.path.abspath(p) for p in args.paths]
    package_repository_manager.clear_caches()
    repo = pyrer.PackageRepository(paths)
    r = random.Random(args.seed)

    names = sorted({f.name for f in iter_package_families(paths=paths)})
    counts = collections.Counter()
    problems = []
    for name in names:
        compare_family(repo, name, paths, None, counts, problems)
        listing = repo.iter_packages(name, None)
        if isinstance(listing, pyrer.NeedsPython) or not listing:
            continue
        versions = [v for _, v, _, _ in listing]
        for _ in range(args.ranges):
            v = r.choice(versions)
            rng = r.choice(["==%s" % v, "%s+" % v, "<%s" % v, v, "%s|%s" % (v, r.choice(versions))])
            compare_family(repo, name, paths, rng, counts, problems)

    print("families: %d" % len(names))
    for k in ("family_match", "family_needs_python", "package_match", "package_needs_python", "BLOCKER"):
        print("  %-22s %d" % (k, counts[k]))
    print("rer stats:", repo.stats())
    for label, detail in problems[: args.show]:
        print("BLOCKER %s\n    %s" % (label, detail))
    return 1 if counts["BLOCKER"] else 0


if __name__ == "__main__":
    sys.exit(main())
