#!/usr/bin/env python3
"""Check that rer's VersionRange strings round-trip through rez.

pyrer passes ``VersionRange::to_string()`` to Python ``load_family``
callbacks as the ``version_range`` hint, and the rez shim hands it to
``iter_packages(range_=...)``. Every such string must parse in rez and
denote the same set of versions as in rer.

Runs the ``#[ignore]``d dump test in ``rer-version``, then checks each
record with ``rez.version.VersionRange``. Exit status 1 on any failure.

Usage::

    python scripts/check_range_roundtrip.py [--n 5000]
"""
import argparse
import json
import os
import subprocess
import sys
import tempfile

from rez.version import Version, VersionRange


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--n", type=int, default=5000)
    ap.add_argument("--show", type=int, default=15)
    args = ap.parse_args()

    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    out = os.path.join(tempfile.mkdtemp(), "ranges.json")
    env = dict(os.environ, RER_RANGE_DUMP=out, RER_RANGE_N=str(args.n))
    subprocess.run(
        ["cargo", "test", "-q", "-p", "rer-version", "--test", "test_range_roundtrip",
         "--", "--ignored"],
        cwd=root, env=env, check=True, stdout=subprocess.DEVNULL)
    with open(out) as f:
        records = json.load(f)

    failures = []
    empty_as_any = 0
    for text, is_empty, members in records:
        if is_empty:
            # An empty range has no string form rez reads as empty ("" is
            # "any" in rez); rer never passes an empty hint (an empty range
            # means the family isn't loaded at all).
            empty_as_any += 1
            continue
        try:
            vr = VersionRange(text)
        except Exception as e:
            failures.append((text, "rez cannot parse: %s" % e))
            continue
        for probe, contained in members:
            if (Version(probe) in vr) != contained:
                failures.append((text, "%s: rer %s, rez %s" % (probe, contained, not contained)))
                break
    print("ranges: %d (empty, skipped: %d)  failures: %d" % (len(records), empty_as_any, len(failures)))
    for text, why in failures[: args.show]:
        print("  %r: %s" % (text, why))
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
