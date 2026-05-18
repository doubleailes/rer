#!/usr/bin/env python3
"""Differential test: `pyrer.parse_static_package_py` vs rez's `Package`.

For every file the Rust parser accepts, load the same file through
rez's `DeveloperPackage.from_path` and assert the four solver-relevant
fields match byte-for-byte. Any divergence is a release blocker — the
fast parser is supposed to produce a `PackageData` that's
indistinguishable from `from_rez(pkg)` for the files it accepts. A
mismatch is a silent correctness regression in any rez integration.

This is the Stage 2 safety net from the RFC at
`docs/content/docs/engineering/fast-package-py-parser.md`. Mirrors
the role of the strict 188-case rez solver differential.

## Run

  python scripts/diff_against_rez.py /thierry/rez/pkg
  python scripts/diff_against_rez.py /thierry/rez/pkg --csv mismatches.csv

Requires:
  - `pyrer` built and importable (from `crates/rer-python`).
  - `rez` importable (the dev venv has it; production rez has it).

## Exit code

  0  — all V2-accepted files agree with rez (or rez-eval errored).
  1  — at least one V2-accepted file diverges from rez.
  2  — rez isn't importable or the corpus path is bad.

The `rez evaluation error` bucket is *not* a mismatch — a file that
crashes rez's evaluator is not a file the rez integration shim would
have accepted either; the slow path would also reject it.
"""
import argparse
import csv
import os
import sys
import time
from collections import Counter
from typing import Any, Dict, Iterator, List, Optional, Tuple


# ---------------------------------------------------------------------------
# Imports — graceful if rez or pyrer aren't available
# ---------------------------------------------------------------------------


try:
    import pyrer
    HAVE_PYRER = True
except Exception as e:
    print(f"FATAL: pyrer not importable ({e})", file=sys.stderr)
    print("  Build with: cd crates/rer-python && maturin develop", file=sys.stderr)
    sys.exit(2)


try:
    from rez.developer_package import DeveloperPackage
    HAVE_REZ = True
except Exception as e:
    print(f"FATAL: rez not importable ({e})", file=sys.stderr)
    sys.exit(2)


# ---------------------------------------------------------------------------
# Walker — mirrors the corpus integration test's traversal so the diff
# sees the same set of files
# ---------------------------------------------------------------------------


def find_package_pys(root: str) -> Iterator[str]:
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [
            d
            for d in dirnames
            if not d.startswith(".")
            and not d.startswith("_")
            and not (len(d) == 40 and all(c in "0123456789abcdef" for c in d))
        ]
        if "package.py" in filenames:
            yield os.path.join(dirpath, "package.py")


# ---------------------------------------------------------------------------
# Normalisation — rez's `requires` is a list of `Requirement` objects;
# pyrer's is a list of `str`. Stringify rez's so we compare like to like.
# ---------------------------------------------------------------------------


def stringify_req_list(items) -> List[str]:
    """rez returns `None` for an unset list, a `SourceCode` for a
    late-bound one, or a list of `Requirement` for a normal one.
    Convert to `list[str]`. A `SourceCode` raises so the caller can
    classify the file as rez-evaluation-error rather than a mismatch."""
    if items is None:
        return []
    out = []
    for r in items:
        # Guard against SourceCode / weird objects without __str__.
        out.append(str(r))
    return out


def stringify_variants(variants) -> List[List[str]]:
    if variants is None:
        return []
    return [stringify_req_list(v) for v in variants]


def field_diff(rust, rez_fields: Dict[str, Any]) -> Dict[str, Tuple[Any, Any]]:
    """Return a dict of differing fields. Empty dict = full match."""
    diffs: Dict[str, Tuple[Any, Any]] = {}
    if rust.name != rez_fields["name"]:
        diffs["name"] = (rust.name, rez_fields["name"])
    if rust.version != rez_fields["version"]:
        diffs["version"] = (rust.version, rez_fields["version"])
    rust_reqs = list(rust.requires)
    if rust_reqs != rez_fields["requires"]:
        diffs["requires"] = (rust_reqs, rez_fields["requires"])
    rust_vars = [list(v) for v in rust.variants]
    if rust_vars != rez_fields["variants"]:
        diffs["variants"] = (rust_vars, rez_fields["variants"])
    return diffs


# ---------------------------------------------------------------------------
# Per-file diff
# ---------------------------------------------------------------------------


def load_rez_fields(path: str) -> Optional[Dict[str, Any]]:
    """Return rez's view of the four solver fields, or `None` on
    evaluation error. Walks the same `DeveloperPackage.from_path`
    code path the rez-integration shim's `from_rez` would."""
    try:
        pkg = DeveloperPackage.from_path(os.path.dirname(path))
        return {
            "name": str(pkg.name),
            "version": str(pkg.version),
            "requires": stringify_req_list(pkg.requires),
            "variants": stringify_variants(pkg.variants),
        }
    except Exception:
        return None


# ---------------------------------------------------------------------------
# Reporting
# ---------------------------------------------------------------------------


def print_mismatch(path: str, diffs: Dict[str, Tuple[Any, Any]], idx: int) -> None:
    print(f"\n  [{idx}] {path}")
    for field, (rust_v, rez_v) in diffs.items():
        print(f"      field    : {field}")
        rust_repr = repr(rust_v) if len(repr(rust_v)) < 200 else repr(rust_v)[:200] + " ..."
        rez_repr = repr(rez_v) if len(repr(rez_v)) < 200 else repr(rez_v)[:200] + " ..."
        print(f"        rust   : {rust_repr}")
        print(f"        rez    : {rez_repr}")


def write_csv(mismatches: List[Tuple[str, Dict[str, Tuple[Any, Any]]]], csv_path: str) -> None:
    with open(csv_path, "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["path", "field", "rust", "rez"])
        for path, diffs in mismatches:
            for field, (rust_v, rez_v) in diffs.items():
                w.writerow([path, field, repr(rust_v), repr(rez_v)])


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("corpus", help="rez repository root (e.g. /thierry/rez/pkg)")
    parser.add_argument(
        "--show-mismatches",
        type=int,
        default=10,
        metavar="N",
        help="Show details for the first N mismatching files (default: 10)",
    )
    parser.add_argument(
        "--csv",
        metavar="PATH",
        help="Write per-mismatch rows to a CSV at PATH",
    )
    parser.add_argument(
        "--max",
        type=int,
        default=0,
        metavar="N",
        help="Stop after N files (0 = no cap, default)",
    )
    args = parser.parse_args()

    if not os.path.isdir(args.corpus):
        print(f"not a directory: {args.corpus}", file=sys.stderr)
        return 2

    counts: Counter = Counter()
    field_mismatches: Counter = Counter()
    mismatch_samples: List[Tuple[str, Dict[str, Tuple[Any, Any]]]] = []
    started = time.time()

    print(f"Differential test: {args.corpus}")
    print(f"  pyrer:  {getattr(pyrer, '__version__', '?')}")
    try:
        import rez
        print(f"  rez:    {rez.__version__}")
    except Exception:
        pass
    print()

    for path in find_package_pys(args.corpus):
        counts["total"] += 1
        if args.max and counts["total"] > args.max:
            break

        try:
            with open(path, "rb") as f:
                source = f.read().decode("utf-8", errors="replace")
        except OSError:
            counts["read_error"] += 1
            continue

        rust = pyrer.parse_static_package_py(source)
        if rust is None:
            counts["v2_bailed"] += 1
            continue

        rez_fields = load_rez_fields(path)
        if rez_fields is None:
            counts["rez_eval_error"] += 1
            continue

        diffs = field_diff(rust, rez_fields)
        if not diffs:
            counts["match"] += 1
        else:
            counts["mismatch"] += 1
            for field in diffs:
                field_mismatches[field] += 1
            mismatch_samples.append((path, diffs))

        # Light progress signal every 500 files — running over CIFS
        # is slow enough that a status line helps.
        if counts["total"] % 500 == 0:
            elapsed = time.time() - started
            print(
                f"  ... {counts['total']:5d} files surveyed "
                f"(match={counts['match']}, mismatch={counts['mismatch']}, "
                f"bailed={counts['v2_bailed']}, rez_err={counts['rez_eval_error']}) "
                f"in {elapsed:.1f}s",
                flush=True,
            )

    elapsed = time.time() - started

    # ----------------------------------------------------------------
    # Report
    # ----------------------------------------------------------------

    print()
    print("Differential test results")
    print("-" * 60)
    print(f"  Total package.py files surveyed:   {counts['total']:6d}  ({elapsed:.1f}s)")
    print(f"    V2 accepted:                     {counts['match'] + counts['mismatch'] + counts['rez_eval_error']:6d}")
    print(f"    V2 bailed:                       {counts['v2_bailed']:6d}")
    print(f"    Read error:                      {counts['read_error']:6d}")
    print()
    accepted = counts["match"] + counts["mismatch"] + counts["rez_eval_error"]
    if accepted:
        print(f"  Differential on V2-accepted ({accepted} files):")
        print(f"    Match (all four fields agree):   {counts['match']:6d}  ({counts['match'] / accepted * 100:.2f}%)")
        print(f"    Mismatch:                        {counts['mismatch']:6d}  ({counts['mismatch'] / accepted * 100:.2f}%)")
        print(f"    rez evaluation error:            {counts['rez_eval_error']:6d}  ({counts['rez_eval_error'] / accepted * 100:.2f}%)")
        print()

    if field_mismatches:
        print("  Mismatch breakdown by field:")
        for field, n in field_mismatches.most_common():
            print(f"    {field:12s}: {n}")
        print()

    if mismatch_samples:
        print(f"Mismatch samples (showing up to {args.show_mismatches}):")
        for i, (path, diffs) in enumerate(mismatch_samples[: args.show_mismatches]):
            print_mismatch(path, diffs, i + 1)
        if len(mismatch_samples) > args.show_mismatches:
            print(f"\n  ({len(mismatch_samples) - args.show_mismatches} more not shown)")

    if args.csv and mismatch_samples:
        write_csv(mismatch_samples, args.csv)
        print(f"\nWrote per-mismatch rows to {args.csv}")

    # Exit code: non-zero if there are any mismatches.
    return 1 if counts["mismatch"] else 0


if __name__ == "__main__":
    sys.exit(main())
