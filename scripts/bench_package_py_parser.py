#!/usr/bin/env python3
"""Micro-benchmark `pyrer.parse_static_package_py` against alternatives.

Quantifies the Stage 3 perf claim in
`docs/content/docs/engineering/fast-package-py-parser.md`: how much
the Rust AST fast-parser saves per file vs. rez's `Package`
evaluator, on the static majority of a real repo.

Measures, per file:

  1. `pyrer.parse_static_package_py(source)`    — the Rust parser.
  2. `pyrer.PackageData.from_rez(fake_pkg)`     — FakeRezPackage:
     attributes are plain Python objects mimicking rez's surface
     (FakeRequirement / FakeVersion need __str__). Lower bound on
     `from_rez` cost; doesn't pay rez's `AttributeForwardMeta`.
  3. `pyrer.PackageData.from_rez(real_pkg)`     — only if `--with-rez`
     is given AND `rez` imports cleanly. Upper bound on `from_rez`
     cost — matches what the Fortiche shim actually pays.
  4. File read (cold cache hint: `--drop-cache` only useful on Linux
     local disk; on CIFS the kernel can't drop SMB caches anyway, so
     this is mostly a warm-cache baseline).

Run without `--corpus` for synthetic shapes; run with
`--corpus /thierry/rez/pkg` for the real Fortiche-shaped numbers.

Examples:

  python scripts/bench_package_py_parser.py
  python scripts/bench_package_py_parser.py --corpus /thierry/rez/pkg
  python scripts/bench_package_py_parser.py --corpus /thierry/rez/pkg --samples 500
"""
import argparse
import os
import pathlib
import sys
import time
import timeit
from typing import Dict, List, Optional, Tuple

import pyrer


# ---------------------------------------------------------------------------
# Synthetic samples — used when --corpus is not given
# ---------------------------------------------------------------------------

SYNTHETIC: Dict[str, str] = {
    "minimal": '''\
name = "foo"
version = "1.0.0"
''',
    "typical": '''\
name = "maya"
version = "2024.0"
description = "Autodesk Maya"
requires = ["python-3", "qt-5"]
variants = [["linux", "python-3.10"], ["linux", "python-3.11"]]

def commands():
    env.PYTHONPATH.append("{root}/python")
    env.PATH.prepend("{root}/bin")
''',
    "with-scope": '''\
# -*- coding: utf-8 -*-
name = "fortichebox"
version = "0.2.0"
requires = ["python-2.7+<3"]

def commands():
    env["FPATH"].append("$SPACE/generic")
    env["FPATH"].append("$SPACE/projects")

with scope("config") as config:
    config.release_packages_path = r"\\\\thierry\\rez\\pkg\\int"

timestamp = 1642007300
''',
    "heavy": '''\
name = "big_package"
version = "3.14.0"
description = "A package with lots of metadata"
authors = ["Alice", "Bob", "Carol"]
requires = ["python-3.11+", "qt-5.15+", "numpy-1.24+", "pandas", "requests"]
variants = [
    ["linux", "x86_64", "python-3.11"],
    ["linux", "x86_64", "python-3.12"],
    ["windows", "x86_64", "python-3.11"],
    ["windows", "x86_64", "python-3.12"],
]
tools = ["bigpkg", "bigpkg-cli", "bigpkg-server"]
tests = {
    "unit": {"command": "pytest tests/"},
    "lint": {"command": "ruff check ."},
}

def commands():
    env.PATH.prepend("{root}/bin")
    env.LD_LIBRARY_PATH.prepend("{root}/lib")
    env.PYTHONPATH.append("{root}/python")
    if defined("DEBUG"):
        env.BIGPKG_DEBUG = "1"

timestamp = 1700000000
format_version = 2
hashed_variants = True
''',
}


# ---------------------------------------------------------------------------
# FakeRezPackage — same as scripts/bench_python_construction.py
# ---------------------------------------------------------------------------


class FakeRequirement:
    __slots__ = ("_s",)

    def __init__(self, s: str) -> None:
        self._s = s

    def __str__(self) -> str:
        return self._s


class FakeVersion:
    __slots__ = ("_s",)

    def __init__(self, s: str) -> None:
        self._s = s

    def __str__(self) -> str:
        return self._s


class FakeRezPackage:
    """Lower-bound stand-in for `rez.packages.Package`. Wraps
    requirement / version strings so `from_rez` pays the per-element
    `__str__` round-trip — but doesn't model rez's
    `AttributeForwardMeta` / late-bound check overhead. Real rez
    Packages are slower than this; use --with-rez for the upper bound.
    """

    __slots__ = ("name", "version", "requires", "variants")

    def __init__(
        self,
        name: str,
        version: str,
        requires: Optional[List[str]],
        variants: Optional[List[List[str]]],
    ) -> None:
        self.name = name
        self.version = FakeVersion(version)
        self.requires = (
            [FakeRequirement(r) for r in requires] if requires else None
        )
        self.variants = (
            [[FakeRequirement(r) for r in v] for v in variants] if variants else None
        )


# ---------------------------------------------------------------------------
# Sampling
# ---------------------------------------------------------------------------


def gather_corpus_samples(root: str, n: int) -> List[pathlib.Path]:
    """Walk `root` and return up to `n` `package.py` paths, sampled
    deterministically (every k-th file) so successive runs of the bench
    against the same corpus see the same files."""
    skip = lambda part: (
        part.startswith(".")
        or part.startswith("_")
        or (len(part) == 40 and all(c in "0123456789abcdef" for c in part))
    )
    paths: List[pathlib.Path] = []
    root_p = pathlib.Path(root)
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if not skip(d)]
        if "package.py" in filenames:
            paths.append(pathlib.Path(dirpath) / "package.py")
    paths.sort()
    if len(paths) <= n:
        return paths
    step = max(len(paths) // n, 1)
    return paths[::step][:n]


def derive_fake(source: str) -> Optional[FakeRezPackage]:
    """Best-effort: pass `source` through the static Rust parser; if
    that accepts it, lift the four fields into a FakeRezPackage so the
    `from_rez` path has a comparable input. Returns None for files the
    Rust parser bails on (those wouldn't be measured against `from_rez`
    in the real shim anyway — the shim falls back to rez for those)."""
    pd = pyrer.parse_static_package_py(source)
    if pd is None:
        return None
    return FakeRezPackage(pd.name, pd.version, pd.requires, pd.variants)


# ---------------------------------------------------------------------------
# Timers
# ---------------------------------------------------------------------------


def time_callable(fn, args_iter, iters: int) -> float:
    """Median μs/call over `iters` repeats, applied to each input in
    `args_iter` once per iteration. Returns the median of the
    per-input medians — robust to outliers."""
    medians: List[float] = []
    for arg in args_iter:
        t = timeit.Timer(lambda a=arg: fn(a))
        times = t.repeat(repeat=5, number=iters)
        medians.append((min(times) / iters) * 1_000_000)  # μs
    medians.sort()
    return medians[len(medians) // 2]  # median of medians


def time_two_arg(fn, args_iter, iters: int) -> float:
    """Same shape as `time_callable` but the timed `fn` takes one
    pre-built object per iteration (used for `from_rez(pkg)`)."""
    return time_callable(fn, args_iter, iters)


# ---------------------------------------------------------------------------
# Real rez (optional)
# ---------------------------------------------------------------------------


def try_import_rez():
    """Try to set up real-rez loading. Returns a callable
    `load(path) -> Package` or None if rez isn't importable."""
    try:
        from rez.developer_package import DeveloperPackage  # type: ignore
    except Exception as e:
        print(f"  (rez not available: {e})", file=sys.stderr)
        return None

    def load(path: pathlib.Path):
        return DeveloperPackage.from_path(str(path.parent))

    return load


# ---------------------------------------------------------------------------
# Reporting
# ---------------------------------------------------------------------------


def print_row(label: str, time_us: Optional[float], baseline: Optional[float] = None) -> None:
    if time_us is None:
        print(f"  {label:42s}   (n/a)")
        return
    if baseline is None:
        print(f"  {label:42s}  {time_us:8.2f} μs")
    else:
        speedup = baseline / time_us if time_us > 0 else float("inf")
        print(f"  {label:42s}  {time_us:8.2f} μs   ({speedup:5.1f}× vs from_rez)")


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--corpus",
        metavar="PATH",
        help="rez repository root to sample real package.py files from",
    )
    parser.add_argument(
        "--samples",
        type=int,
        default=100,
        help="files to sample from --corpus (default: 100)",
    )
    parser.add_argument(
        "--iters",
        type=int,
        default=200,
        help="`timeit` `number` argument per repeat (default: 200)",
    )
    parser.add_argument(
        "--with-rez",
        action="store_true",
        help="Also time real rez.Package loading (requires rez installed)",
    )
    args = parser.parse_args()

    # Gather sources.
    if args.corpus:
        if not os.path.isdir(args.corpus):
            print(f"not a directory: {args.corpus}", file=sys.stderr)
            return 2
        print(f"Sampling up to {args.samples} files from {args.corpus} ...")
        paths = gather_corpus_samples(args.corpus, args.samples)
        sources: List[Tuple[str, pathlib.Path]] = []
        for p in paths:
            try:
                with open(p, "rb") as f:
                    sources.append((f.read().decode("utf-8", errors="replace"), p))
            except OSError:
                continue
        print(f"  read {len(sources)} files")
    else:
        sources = [(src, pathlib.Path(f"<{name}>")) for name, src in SYNTHETIC.items()]
        print(f"Using {len(sources)} synthetic samples (no --corpus)")
    print()

    # ------------------------------------------------------------------
    # Timing #1: Rust AST fast-parser
    # ------------------------------------------------------------------
    static_sources = [s for s, _ in sources]
    static_accept_count = sum(
        1 for s in static_sources if pyrer.parse_static_package_py(s) is not None
    )
    t_static = time_callable(
        pyrer.parse_static_package_py, static_sources, args.iters
    )

    # ------------------------------------------------------------------
    # Timing #2: from_rez(FakeRezPackage) — lower bound
    # ------------------------------------------------------------------
    fakes = [derive_fake(s) for s in static_sources]
    fakes = [f for f in fakes if f is not None]
    if fakes:
        t_from_rez_fake = time_two_arg(
            pyrer.PackageData.from_rez, fakes, args.iters
        )
    else:
        t_from_rez_fake = None

    # ------------------------------------------------------------------
    # Timing #3a: from_rez(real rez Package) — post-load only (μs to
    # walk an already-loaded Package's four attributes). Not a fair
    # comparison to the Rust parser by itself — see #3b.
    # ------------------------------------------------------------------
    t_from_rez_real: Optional[float] = None
    # Timing #3b: full load — `DeveloperPackage.from_path(dir) +
    # from_rez(pkg)`. The production-realistic equivalent to the Rust
    # parser's `open+read+parse_static_package_py(source)`.
    t_full_load_real: Optional[float] = None
    # Timing #3c: full load via the Rust parser path — `open + read +
    # parse_static_package_py`. Apples-to-apples to 3b.
    t_full_load_rust: Optional[float] = None

    if args.with_rez:
        load = try_import_rez()
        if load is not None and args.corpus:
            print("Loading real rez Packages ...")
            real_pkgs = []
            real_paths = []
            for s, p in sources:
                try:
                    real_pkgs.append(load(p))
                    real_paths.append(p)
                except Exception:
                    continue
                if len(real_pkgs) >= min(args.samples, 30):
                    break
            if real_pkgs:
                t_from_rez_real = time_two_arg(
                    pyrer.PackageData.from_rez, real_pkgs, args.iters
                )
                # 3b: full load — does its own DeveloperPackage.from_path
                # call every iteration.
                def _full_load_rez(p):
                    pkg = load(p)
                    return pyrer.PackageData.from_rez(pkg)
                t_full_load_real = time_two_arg(
                    _full_load_rez, real_paths, max(args.iters // 10, 5)
                )
                # 3c: full load via Rust parser — open+read+parse every
                # iteration. Same set of paths.
                def _full_load_rust(p):
                    with open(p, "rb") as f:
                        src = f.read().decode("utf-8", errors="replace")
                    return pyrer.parse_static_package_py(src)
                t_full_load_rust = time_two_arg(
                    _full_load_rust, real_paths, max(args.iters // 10, 5)
                )

    # ------------------------------------------------------------------
    # Timing #4: pure file-read baseline (so the reader can see what
    # fraction of "open + parse" is I/O vs CPU).
    # ------------------------------------------------------------------
    t_file_read: Optional[float] = None
    if args.corpus:
        paths_only = [p for _, p in sources]

        def _read(p):
            with open(p, "rb") as f:
                f.read()

        t_file_read = time_two_arg(_read, paths_only, max(args.iters // 10, 5))

    # ------------------------------------------------------------------
    # Report
    # ------------------------------------------------------------------
    print("Per-file timings (median μs/file, best of 5)")
    print("-" * 78)
    print("In-memory comparison (source / pkg pre-built; not production):")
    baseline = t_from_rez_real if t_from_rez_real else t_from_rez_fake
    print_row("  parse_static_package_py(source)", t_static, baseline)
    print_row("  from_rez(fake_pkg)", t_from_rez_fake, t_from_rez_fake)
    print_row("  from_rez(real rez Package, preloaded)", t_from_rez_real, baseline)
    print_row("  file read only (open+read)", t_file_read, None)
    print()
    print("Full-load comparison (apples to apples — what the shim actually pays):")
    print_row("  open+read+parse_static_package_py", t_full_load_rust, t_full_load_real)
    print_row("  DeveloperPackage.from_path + from_rez", t_full_load_real, t_full_load_real)

    print()
    print(f"Rust parser accept rate on the sample: "
          f"{static_accept_count}/{len(static_sources)} "
          f"({static_accept_count / max(len(static_sources), 1) * 100:.1f}%)")
    print()
    print("Notes:")
    print("  - `from_rez(fake_pkg)` is a LOWER BOUND on the real cost. It")
    print("    doesn't pay rez's AttributeForwardMeta / late-bound checks.")
    print("  - `from_rez(real rez)` (when --with-rez is given AND rez is")
    print("    installed) is the actual shim cost. Run that for the number")
    print("    that matters in production.")
    print("  - File-read is the I/O floor — on cold CIFS it can dominate")
    print("    the parse step. The Rust parser saves CPU; load_family")
    print("    saves I/O.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
