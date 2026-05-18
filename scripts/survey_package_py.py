#!/usr/bin/env python3
"""Stage 1 corpus survey for the Rust `package.py` fast-parser RFC.

Walks a tree of rez `package.py` files and classifies each one as
*fast-parseable* (the four solver-relevant fields `name`, `version`,
`requires`, `variants` are all literal assignments and the file has
no disqualifying top-level statements) or *not* — with the reasons
listed.

The output is the go/no-go signal for Stages 2-3 of the RFC at
`docs/content/docs/engineering/fast-package-py-parser.md`. If
≥70% of a real studio repo is fast-parseable, the Rust parser is
worth building. Below 40%, prefer a parsed-package cache instead.

The classifier recognises two ignorable patterns the Rust parser
will also need to handle:

  - `def commands()`, `def pre_commands()`, etc. — function bodies
    that affect runtime environment, not the solve.
  - `with scope("config")` / `with scope("build")` — rez's
    declarative DSL for non-solver config. Body writes attributes
    of the `as`-name, never solver fields.

Anything else at module scope (top-level `if`, `for`, `try`,
`import`, `class`, `@early`/`@late` on a solver field) disqualifies
the file from the fast path.

Pure-stdlib: no pyrer / rez / rustpython-parser dependency. Uses
Python's `ast` module — meaning the parser used here is the same
shape as the one Rust's `rustpython-parser` would produce, so the
classification rules carry over cleanly.

Usage:
  python scripts/survey_package_py.py /path/to/rez/repo
  python scripts/survey_package_py.py /path/to/rez/repo --csv out.csv
  python scripts/survey_package_py.py /path/to/rez/repo --show-examples 5

`Resource temporarily unavailable` errors from a CIFS mount are
caught and counted under `io-error` — they don't crash the survey.
"""
import argparse
import ast
import os
import sys
from collections import Counter
from dataclasses import dataclass, field
from typing import Iterable, List, Optional, Tuple


SOLVER_FIELDS = ("name", "version", "requires", "variants")


@dataclass
class FileVerdict:
    path: str
    fast_parseable: bool
    reasons: List[str] = field(default_factory=list)
    # Which solver fields were found, and which were literal-static.
    fields_present: List[str] = field(default_factory=list)
    fields_static: List[str] = field(default_factory=list)


# ---------------------------------------------------------------------------
# AST classification
# ---------------------------------------------------------------------------


def _is_str_literal(node: ast.AST) -> bool:
    return isinstance(node, ast.Constant) and isinstance(node.value, str)


def _is_list_of_str_literal(node: ast.AST) -> bool:
    """List or Tuple of string literals — what `requires` looks like."""
    if not isinstance(node, (ast.List, ast.Tuple)):
        return False
    return all(_is_str_literal(elt) for elt in node.elts)


def _is_list_of_list_of_str_literal(node: ast.AST) -> bool:
    """List of List/Tuple of string literals — what `variants` looks like."""
    if not isinstance(node, (ast.List, ast.Tuple)):
        return False
    return all(_is_list_of_str_literal(inner) for inner in node.elts)


def _assign_targets(node: ast.Assign) -> List[str]:
    """Names assigned to in a single Assign statement (handles a, b = ...)."""
    names = []
    for target in node.targets:
        if isinstance(target, ast.Name):
            names.append(target.id)
        elif isinstance(target, (ast.Tuple, ast.List)):
            for elt in target.elts:
                if isinstance(elt, ast.Name):
                    names.append(elt.id)
    return names


def _function_is_early_or_late(func: ast.FunctionDef) -> bool:
    """`@early()` / `@late()` decorated function — dynamic value."""
    for deco in func.decorator_list:
        # `@early` (bare name) or `@early()` (call)
        target = deco.func if isinstance(deco, ast.Call) else deco
        if isinstance(target, ast.Name) and target.id in ("early", "late"):
            return True
    return False


def _is_scope_with(node: ast.With) -> bool:
    """rez's declarative DSL: `with scope("config") as config: ...`. The
    body only writes to attributes of the `as`-name (which never overlaps
    a solver field), so the whole block is solver-irrelevant — treat it
    the same way we treat `def commands(...)`. All `with` items must be
    `scope(...)` calls for this to apply."""
    if not node.items:
        return False
    for item in node.items:
        ctx = item.context_expr
        if not (
            isinstance(ctx, ast.Call)
            and isinstance(ctx.func, ast.Name)
            and ctx.func.id == "scope"
        ):
            return False
    return True


def classify(source: str, path: str) -> FileVerdict:
    """Walk the module's top-level statements and decide if the four solver
    fields are all literal-static, and whether anything else at module
    level would force a bail."""
    verdict = FileVerdict(path=path, fast_parseable=False)

    try:
        tree = ast.parse(source, filename=path)
    except SyntaxError as e:
        verdict.reasons.append(f"syntax-error:{e.msg}")
        return verdict

    # Track per-field: present anywhere, and whether the last assignment
    # / definition for that field is static. We treat *any* dynamic
    # observation as poisoning the field — rebinds to literal afterwards
    # don't help (we'd still need to evaluate the dynamic branch).
    field_present = {f: False for f in SOLVER_FIELDS}
    field_static = {f: False for f in SOLVER_FIELDS}
    field_poisoned = {f: False for f in SOLVER_FIELDS}

    for i, stmt in enumerate(tree.body):
        # Module docstring — Expr(Constant(str)) as first statement.
        if (
            i == 0
            and isinstance(stmt, ast.Expr)
            and _is_str_literal(stmt.value)
        ):
            continue

        # `import x` / `from x import y` — possibly load-bearing.
        if isinstance(stmt, (ast.Import, ast.ImportFrom)):
            verdict.reasons.append("imports")
            continue

        # Special case: `with scope("config") as config: ...` — rez's
        # declarative DSL for setting non-solver-relevant config (paths,
        # release settings). Body writes attributes of the `as`-name,
        # never module-level solver fields. Treat as ignorable; still
        # walk in defensively to catch a pathological body that does
        # touch a solver field.
        if isinstance(stmt, ast.With) and _is_scope_with(stmt):
            for descendant in ast.walk(stmt):
                if isinstance(descendant, ast.Assign):
                    for n in _assign_targets(descendant):
                        if n in SOLVER_FIELDS:
                            field_poisoned[n] = True
                            verdict.reasons.append(f"scope-with-assigns-{n}")
            continue

        # Top-level control flow — can't statically know which branch wins.
        if isinstance(stmt, (ast.If, ast.For, ast.While, ast.Try, ast.With)):
            verdict.reasons.append(f"top-level-{type(stmt).__name__.lower()}")
            # Also need to walk into it to see if it touches solver fields,
            # but the bail decision is already made — record the reason and
            # mark any solver field touched as poisoned.
            for descendant in ast.walk(stmt):
                if isinstance(descendant, ast.Assign):
                    for n in _assign_targets(descendant):
                        if n in SOLVER_FIELDS:
                            field_poisoned[n] = True
            continue

        # `def commands(): ...` etc., or `@early` def for a solver field.
        if isinstance(stmt, ast.FunctionDef):
            if stmt.name in SOLVER_FIELDS:
                if _function_is_early_or_late(stmt):
                    verdict.reasons.append(f"dynamic-{stmt.name}")
                    field_poisoned[stmt.name] = True
                else:
                    # Plain `def requires():` without @early is unusual but
                    # functionally dynamic — bail.
                    verdict.reasons.append(f"function-def-{stmt.name}")
                    field_poisoned[stmt.name] = True
            # `def commands(...)`, `def build(...)`, etc. — not solver-
            # relevant, ignore.
            continue

        # AsyncFunctionDef, ClassDef — uncommon in package.py; bail.
        if isinstance(stmt, (ast.AsyncFunctionDef, ast.ClassDef)):
            verdict.reasons.append(f"top-level-{type(stmt).__name__.lower()}")
            continue

        # Plain assignment.
        if isinstance(stmt, ast.Assign):
            targets = _assign_targets(stmt)
            for n in targets:
                if n not in SOLVER_FIELDS:
                    continue
                field_present[n] = True
                if n == "name" and _is_str_literal(stmt.value):
                    field_static[n] = True
                elif n == "version" and _is_str_literal(stmt.value):
                    field_static[n] = True
                elif n == "requires" and _is_list_of_str_literal(stmt.value):
                    field_static[n] = True
                elif n == "variants" and _is_list_of_list_of_str_literal(
                    stmt.value
                ):
                    field_static[n] = True
                else:
                    field_poisoned[n] = True
            continue

        # `requires: list[str] = [...]` — annotated assignment.
        if isinstance(stmt, ast.AnnAssign) and isinstance(stmt.target, ast.Name):
            n = stmt.target.id
            if n in SOLVER_FIELDS:
                field_present[n] = True
                if stmt.value is None:
                    # type-only annotation, no value bound.
                    field_poisoned[n] = True
                elif n == "name" and _is_str_literal(stmt.value):
                    field_static[n] = True
                elif n == "version" and _is_str_literal(stmt.value):
                    field_static[n] = True
                elif n == "requires" and _is_list_of_str_literal(stmt.value):
                    field_static[n] = True
                elif n == "variants" and _is_list_of_list_of_str_literal(
                    stmt.value
                ):
                    field_static[n] = True
                else:
                    field_poisoned[n] = True
            continue

        # `name += "foo"`, etc. — AugAssign on a solver field is dynamic.
        if isinstance(stmt, ast.AugAssign) and isinstance(stmt.target, ast.Name):
            if stmt.target.id in SOLVER_FIELDS:
                field_poisoned[stmt.target.id] = True
            continue

        # Expression statements (function calls at module level): suspicious.
        if isinstance(stmt, ast.Expr):
            verdict.reasons.append("top-level-expr")
            continue

        # Catch-all for anything we didn't list.
        verdict.reasons.append(f"unrecognised-{type(stmt).__name__.lower()}")

    # name & version are required; requires & variants default to empty.
    must_have = ("name", "version")
    static_ok = True
    for f in must_have:
        if not field_present[f]:
            verdict.reasons.append(f"missing-{f}")
            static_ok = False
        elif field_poisoned[f] or not field_static[f]:
            verdict.reasons.append(f"non-literal-{f}")
            static_ok = False
    for f in ("requires", "variants"):
        if field_present[f] and (field_poisoned[f] or not field_static[f]):
            verdict.reasons.append(f"non-literal-{f}")
            static_ok = False

    verdict.fields_present = [f for f, p in field_present.items() if p]
    verdict.fields_static = [f for f, s in field_static.items() if s]
    verdict.fast_parseable = static_ok and not verdict.reasons
    return verdict


# ---------------------------------------------------------------------------
# Walking the repository
# ---------------------------------------------------------------------------


def find_package_pys(root: str) -> Iterable[str]:
    """Yield every `package.py` under `root`. Skips dot-directories and
    rez variant subdirs (40-char hex hashes) since those re-shadow the
    parent's package.py with the same content."""
    for dirpath, dirnames, filenames in os.walk(root, followlinks=False):
        # Prune hidden + rez variant subdirs.
        dirnames[:] = [
            d for d in dirnames
            if not d.startswith(".")
            and not d.startswith("_")
            and not _looks_like_variant_hash(d)
        ]
        if "package.py" in filenames:
            yield os.path.join(dirpath, "package.py")


def _looks_like_variant_hash(name: str) -> bool:
    return len(name) == 40 and all(c in "0123456789abcdef" for c in name)


def survey(root: str) -> Tuple[List[FileVerdict], int]:
    """Walk and classify every package.py under `root`."""
    verdicts: List[FileVerdict] = []
    io_errors = 0
    for path in find_package_pys(root):
        try:
            with open(path, "rb") as f:
                source = f.read().decode("utf-8", errors="replace")
        except OSError:
            io_errors += 1
            continue
        verdicts.append(classify(source, path))
    return verdicts, io_errors


# ---------------------------------------------------------------------------
# Reporting
# ---------------------------------------------------------------------------


def report(
    verdicts: List[FileVerdict],
    io_errors: int,
    show_examples: int,
) -> None:
    n = len(verdicts)
    if n == 0 and io_errors == 0:
        print("No package.py files found.", file=sys.stderr)
        return

    fast = [v for v in verdicts if v.fast_parseable]
    slow = [v for v in verdicts if not v.fast_parseable]

    print(f"Surveyed: {n} package.py files")
    if io_errors:
        print(f"          {io_errors} I/O errors (skipped — likely CIFS flakiness)")
    print()
    print(f"  fast-parseable:     {len(fast):6d}  ({len(fast) / n * 100:5.1f}%)")
    print(f"  not fast-parseable: {len(slow):6d}  ({len(slow) / n * 100:5.1f}%)")
    print()

    # Reason histogram (a file can show up in multiple buckets).
    reason_counts: Counter = Counter()
    for v in slow:
        for r in set(v.reasons):  # dedupe per-file (multiple imports → one count)
            reason_counts[r] += 1

    if reason_counts:
        print("Reasons (files can disqualify on multiple — counts are independent):")
        print("-" * 60)
        for reason, count in reason_counts.most_common():
            print(f"  {reason:30s}  {count:6d}  ({count / n * 100:5.1f}% of total)")
        print()

    if show_examples > 0:
        print(f"Examples (up to {show_examples} per bucket):")
        print("-" * 60)
        print(f"\n  fast-parseable:")
        for v in fast[:show_examples]:
            print(f"    {v.path}")
        for reason in sorted({r for v in slow for r in v.reasons}):
            print(f"\n  {reason}:")
            examples = [v.path for v in slow if reason in v.reasons][:show_examples]
            for p in examples:
                print(f"    {p}")
        print()

    # The go/no-go decision rule from the RFC.
    pct = len(fast) / n * 100 if n else 0
    print("Go / no-go (from the RFC):")
    print("-" * 60)
    if pct >= 70:
        print(f"  ≥ 70% fast-parseable ({pct:.1f}%) — PROCEED to Stages 2-3.")
    elif pct >= 40:
        print(
            f"  {pct:.1f}% fast-parseable — marginal. Worth a discussion about"
            "\n  which patterns to expand coverage to, or whether the"
            "\n  parsed-package cache alternative pays off better."
        )
    else:
        print(
            f"  < 40% fast-parseable ({pct:.1f}%) — don't build the parser."
            "\n  The slow-path fallback would dominate. Pivot to the"
            "\n  parsed-package cache alternative in the RFC."
        )


def write_csv(verdicts: List[FileVerdict], csv_path: str) -> None:
    import csv

    with open(csv_path, "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(
            ["path", "fast_parseable", "reasons", "fields_present", "fields_static"]
        )
        for v in verdicts:
            w.writerow(
                [
                    v.path,
                    int(v.fast_parseable),
                    ";".join(sorted(set(v.reasons))),
                    ";".join(v.fields_present),
                    ";".join(v.fields_static),
                ]
            )


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("root", help="rez repository root (e.g. /thierry/rez/pkg)")
    parser.add_argument(
        "--csv",
        metavar="PATH",
        help="Write per-file verdicts to a CSV at PATH (optional)",
    )
    parser.add_argument(
        "--show-examples",
        type=int,
        default=0,
        metavar="N",
        help="Show up to N example file paths per bucket (default: 0)",
    )
    args = parser.parse_args()

    if not os.path.isdir(args.root):
        print(f"not a directory: {args.root}", file=sys.stderr)
        return 2

    verdicts, io_errors = survey(args.root)
    report(verdicts, io_errors, args.show_examples)

    if args.csv:
        write_csv(verdicts, args.csv)
        print(f"\nWrote per-file verdicts to {args.csv}")

    return 0


if __name__ == "__main__":
    sys.exit(main())
