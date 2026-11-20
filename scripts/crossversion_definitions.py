#!/usr/bin/env python3
"""Check that rez loads the same package definitions identically across
Python versions.

rer reproduces rez's ``SourceCode`` text, which comes from
``inspect.getsourcelines`` — stdlib code that can change between Python
releases. rer follows CPython 3.13; this script shows whether the other
versions rez supports (>= 3.8) agree on a corpus.

Dump with each interpreter (rez importable, e.g. via ``PYTHONPATH``)::

    PYTHONPATH=rez/src python3.8  scripts/crossversion_definitions.py dump out-3.8.json ROOT...
    PYTHONPATH=rez/src python3.13 scripts/crossversion_definitions.py dump out-3.13.json ROOT...

then compare::

    python scripts/crossversion_definitions.py compare out-3.8.json out-3.13.json

Exit status 1 if any file differs between versions.
"""
import json
import os
import sys


def norm(v):
    from rez.utils.sourcecode import SourceCode

    if isinstance(v, dict):
        return ["dict", [[norm(k), norm(x)] for k, x in v.items()]]
    if isinstance(v, (list, tuple)):
        return [type(v).__name__, [norm(x) for x in v]]
    if isinstance(v, SourceCode):
        return ["SourceCode", v.source, v.funcname, v.eval_as_function, repr(v.decorators)]
    if v is None or isinstance(v, (bool, int, float, str)):
        return [type(v).__name__, repr(v)]
    return ["object", type(v).__name__]


def dump(out, roots):
    from rez.serialise import FileFormat, load_from_file

    result = {}
    for root in roots:
        for dirpath, dirnames, filenames in os.walk(root):
            dirnames.sort()
            if "package.py" not in filenames:
                continue
            path = os.path.join(dirpath, "package.py")
            try:
                data = load_from_file(path, FileFormat.py, disable_memcache=True)
                result[path] = norm(data)
            except Exception as e:
                result[path] = ["error", type(e).__name__]
    with open(out, "w") as f:
        json.dump({"python": sys.version.split()[0], "files": result}, f)
    print("%s: %d files" % (sys.version.split()[0], len(result)))


def compare(paths):
    dumps = []
    for p in paths:
        with open(p) as f:
            dumps.append(json.load(f))
    base = dumps[0]
    bad = 0
    for other in dumps[1:]:
        diffs = [k for k in base["files"] if base["files"][k] != other["files"].get(k)]
        print("%s vs %s: %d of %d files differ" % (
            base["python"], other["python"], len(diffs), len(base["files"])))
        for k in diffs[:10]:
            print("   ", k)
        bad += len(diffs)
    return 1 if bad else 0


if __name__ == "__main__":
    if sys.argv[1] == "dump":
        dump(sys.argv[2], sys.argv[3:])
    elif sys.argv[1] == "compare":
        sys.exit(compare(sys.argv[2:]))
    else:
        sys.exit(__doc__)
