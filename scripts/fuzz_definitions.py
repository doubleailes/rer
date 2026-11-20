#!/usr/bin/env python3
"""Seeded fuzzer for the rez-vs-rer definition differential.

Generates random ``package.py`` files that stress what the Rust loader must
reproduce exactly — function-body source extraction (comments at odd
columns, blank lines, nested blocks, column-0 lines inside strings,
decorators, one-liners), ``with scope`` capture, and literals — writes them
to a temp dir, and runs ``diff_definitions.compare_file`` on each.

Usage::

    python scripts/fuzz_definitions.py [--n 2000] [--seed 0] [--keep DIR]

Exit status is 1 if any file is a blocker (MISMATCH / rer_static_rez_error).
"""
import argparse
import collections
import os
import random
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from diff_definitions import compare_file  # noqa: E402


def rand_literal(r, depth=0):
    choices = ["str", "int", "float", "bool", "none"]
    if depth < 2:
        choices += ["list", "tuple", "dict"]
    kind = r.choice(choices)
    if kind == "str":
        return r.choice(["'a'", '"b-1+<2"', "'''x\ny'''", "r'\\d'", "'\\t'", "'é'", "''", '"q\\"q"'])
    if kind == "int":
        return r.choice(["0", "1", "-5", "1_000", "0xff", "+7"])
    if kind == "float":
        return r.choice(["1.5", "-0.5", "1e3", "2.", ".25"])
    if kind == "bool":
        return r.choice(["True", "False"])
    if kind == "none":
        return "None"
    items = [rand_literal(r, depth + 1) for _ in range(r.randint(0, 3))]
    if kind == "list":
        return "[" + ", ".join(items) + (r.choice(["", ","]) if items else "") + "]"
    if kind == "tuple":
        if len(items) == 1:
            return "(" + items[0] + ",)"
        return "(" + ", ".join(items) + ")"
    keys = ["'k%d'" % i for i in range(len(items))]
    return "{" + ", ".join("%s: %s" % kv for kv in zip(keys, items)) + "}"


def rand_body_line(r, indent):
    pad = " " * indent
    kind = r.choice(["stmt", "stmt", "stmt", "comment", "blank", "block", "string", "inline"])
    if kind == "stmt":
        return [pad + r.choice(["env.A = 1", "env.PATH.append('{root}/bin')", "x = [1,\n%s    2]" % pad, "pass"])]
    if kind == "inline":
        return [pad + "env.B = 2  # inline"]
    if kind == "comment":
        col = r.choice([0, 1, 2, indent, indent + 4])
        return [" " * col + "# c"]
    if kind == "blank":
        return [r.choice(["", "  ", pad])]
    if kind == "string":
        return [pad + 's = """', "col0 text", '    indented"""']
    lines = [pad + r.choice(["if x:", "for i in y:", "def inner():", "with z:"])]
    for _ in range(r.randint(1, 3)):
        lines += rand_body_line(r, indent + 4)
    lines.append(pad + "    env.N = 1")
    return lines


def rand_function(r):
    name = r.choice(["commands", "pre_commands", "post_commands", "tools", "helper", "description"])
    decorators = []
    if name in ("tools", "description") and r.random() < 0.7:
        decorators.append("@late()")
    if r.random() < 0.3:
        decorators.insert(r.randint(0, len(decorators)), "@include('mod_a', 'mod_b')")
    sig = r.choice(["():", "(a=1):", "(\n    a=1):"])
    if r.random() < 0.08:
        return decorators + ["def %s(): env.X = 1" % name]
    lines = decorators + ["def %s%s" % (name, sig)]
    for _ in range(r.randint(1, 6)):
        lines += rand_body_line(r, 4)
    lines.append("    env.LAST = 1")
    if r.random() < 0.15:
        lines.append(r.choice(["    # trailing", "  # trailing2", "# trailing0"]))
    return lines


def rand_scope(r):
    lines = ["with scope(%r)%s:" % (r.choice(["config", "other"]), r.choice([" as c", ""]))]
    for _ in range(r.randint(1, 4)):
        if lines[0].endswith("as c:") and r.random() < 0.6:
            lines.append("    c.%s = %s" % (r.choice(["a", "b.c", "b.d.e"]), rand_literal(r)))
        else:
            lines.append("    %s = %s" % (r.choice(["x", "y", "name"]), rand_literal(r)))
    return lines


def rand_package(r):
    lines = ["name = 'pkg'", "version = '1.0'"]
    for _ in range(r.randint(1, 8)):
        kind = r.choice(["assign", "assign", "func", "func", "scope", "blank", "comment"])
        if kind == "assign":
            lines.append("%s = %s" % (r.choice(["requires", "x", "_p", "tools", "description", "y"]), rand_literal(r)))
        elif kind == "func":
            lines += rand_function(r)
        elif kind == "scope":
            lines += rand_scope(r)
        elif kind == "blank":
            lines.append("")
        else:
            lines.append("# top-level comment")
    text = "\n".join(lines) + "\n"
    if r.random() < 0.1:
        text = text.replace("\n", "\r\n")
    return text


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--n", type=int, default=2000)
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--keep", help="write generated packages here instead of a temp dir")
    args = ap.parse_args()

    r = random.Random(args.seed)
    root = args.keep or tempfile.mkdtemp(prefix="rer-fuzz-")
    counts = collections.Counter()
    blockers = []
    for i in range(args.n):
        d = os.path.join(root, "p%05d" % i)
        os.makedirs(d, exist_ok=True)
        path = os.path.join(d, "package.py")
        with open(path, "w", newline="") as f:
            f.write(rand_package(r))
        outcome, reason, detail = compare_file(path)
        counts[outcome if outcome != "needs_python" else "needs_python:" + reason] += 1
        if outcome in ("MISMATCH", "rer_static_rez_error"):
            blockers.append((path, outcome, detail))
    for k, v in sorted(counts.items()):
        print("%-36s %d" % (k, v))
    for path, outcome, detail in blockers[:20]:
        print("%s %s\n    %s" % (outcome, path, detail))
    print("blockers: %d (files in %s)" % (len(blockers), root))
    return 1 if blockers else 0


if __name__ == "__main__":
    sys.exit(main())
