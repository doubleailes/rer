#!/usr/bin/env python3
"""Differential test: rez vs rer on full package-definition loading.

For every ``package.py`` / ``package.yaml`` under the given roots, load it
with rez (``rez.serialise.load_from_file``) and with rer
(``pyrer.load_package_definition``) and compare the resulting data dicts:
keys, key order, value types and values, and ``SourceCode`` state
(source text, funcname, eval_as_function, decorators).

Outcomes per file:

- ``match``         rer produced exactly rez's dict.
- ``needs_python``  rer declined (counted by reason); rez handles the file.
- ``MISMATCH``      rer produced a dict that differs from rez's. Release blocker.
- ``rer_static_rez_error``  rer produced a dict where rez raises. Release blocker.

Exit status is 1 if any blocker is found.

Usage::

    python scripts/diff_definitions.py ROOT [ROOT ...] [--show N] [--json OUT]

Requires ``rez`` and ``pyrer`` importable. Note that rez *executes* every
``package.py`` it loads, as it would in production.
"""
import argparse
import collections
import json
import os
import sys

import pyrer
from rez.serialise import FileFormat, load_from_file
from rez.utils.sourcecode import SourceCode

FILENAMES = {"package.py": "py", "package.yaml": "yaml"}


def iter_definition_files(roots):
    for root in roots:
        if os.path.isfile(root):
            yield root
            continue
        for dirpath, dirnames, filenames in os.walk(root):
            dirnames.sort()
            for fn in sorted(filenames):
                if fn in FILENAMES:
                    yield os.path.join(dirpath, fn)


def norm(v):
    """Comparable, order-preserving form of a loaded value."""
    if isinstance(v, dict):
        return ("dict", [(norm(k), norm(x)) for k, x in v.items()])
    if isinstance(v, list):
        return ("list", [norm(x) for x in v])
    if isinstance(v, tuple):
        return ("tuple", [norm(x) for x in v])
    if isinstance(v, (SourceCode, pyrer.SourceCodeData)):
        decorators = [sorted((k, norm(x)) for k, x in d.items()) for d in v.decorators]
        return ("SourceCode", v.source, v.funcname, v.eval_as_function, decorators)
    if v is None or isinstance(v, (bool, int, float, str)):
        if isinstance(v, float) and v != v:
            return ("float", "nan")
        return (type(v).__name__, v)
    return ("object", type(v).__name__)


def first_difference(a, b, path="data"):
    if a == b:
        return None
    if a[0] != b[0]:
        return "%s: type %s (rez) vs %s (rer)" % (path, a[0], b[0])
    if a[0] in ("dict",):
        ka = [k for k, _ in a[1]]
        kb = [k for k, _ in b[1]]
        if ka != kb:
            return "%s: keys %r (rez) vs %r (rer)" % (path, ka, kb)
        for (k, x), (_, y) in zip(a[1], b[1]):
            d = first_difference(x, y, "%s[%r]" % (path, k[1] if len(k) > 1 else k))
            if d:
                return d
    if a[0] in ("list", "tuple"):
        if len(a[1]) != len(b[1]):
            return "%s: length %d (rez) vs %d (rer)" % (path, len(a[1]), len(b[1]))
        for i, (x, y) in enumerate(zip(a[1], b[1])):
            d = first_difference(x, y, "%s[%d]" % (path, i))
            if d:
                return d
    return "%s: %r (rez) vs %r (rer)" % (path, a[1:], b[1:])


def compare_file(path):
    fmt = FILENAMES.get(os.path.basename(path), "py")
    with open(path, "rb") as f:
        raw = f.read()
    rer = pyrer.load_package_definition(raw, fmt)

    try:
        rez_data = load_from_file(
            path, FileFormat.py if fmt == "py" else FileFormat.yaml,
            disable_memcache=True)
        rez_err = None
    except BaseException as e:  # rez raises a variety of types
        rez_data, rez_err = None, "%s: %s" % (type(e).__name__, str(e).splitlines()[0] if str(e) else "")

    if isinstance(rer, pyrer.NeedsPython):
        return "needs_python", rer.reason, "%s (line %s): %s" % (rer.reason, rer.line, rer.detail)
    if rez_err is not None:
        return "rer_static_rez_error", None, rez_err
    a, b = norm(rez_data), norm(rer)
    if a == b:
        return "match", None, None
    return "MISMATCH", None, first_difference(a, b)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("roots", nargs="+")
    ap.add_argument("--show", type=int, default=20, help="examples to print per bucket")
    ap.add_argument("--json", help="write per-file results here")
    args = ap.parse_args()

    counts = collections.Counter()
    reasons = collections.Counter()
    examples = collections.defaultdict(list)
    rows = []
    for path in iter_definition_files(args.roots):
        outcome, reason, detail = compare_file(path)
        counts[outcome] += 1
        if reason:
            reasons[reason] += 1
        key = outcome if outcome != "needs_python" else "needs_python:" + reason
        if len(examples[key]) < args.show:
            examples[key].append((path, detail))
        rows.append({"path": path, "outcome": outcome, "reason": reason, "detail": detail})

    total = sum(counts.values())
    print("files: %d" % total)
    for k in ("match", "needs_python", "MISMATCH", "rer_static_rez_error"):
        print("  %-22s %7d  (%.1f%%)" % (k, counts[k], 100.0 * counts[k] / total if total else 0))
    for r, n in reasons.most_common():
        print("    needs_python %-18s %6d" % (r, n))
    for key in sorted(examples):
        if key == "match":
            continue
        print("\n== %s" % key)
        for path, detail in examples[key]:
            print("  %s\n      %s" % (path, detail))
    if args.json:
        with open(args.json, "w") as f:
            json.dump(rows, f, indent=1)
    blockers = counts["MISMATCH"] + counts["rer_static_rez_error"]
    print("\nblockers: %d" % blockers)
    return 1 if blockers else 0


if __name__ == "__main__":
    sys.exit(main())
