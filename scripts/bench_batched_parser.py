#!/usr/bin/env python3
"""Measure `pyrer.parse_static_packages_py` (batched, Rayon-parallel)
against the serial-Python equivalent on real `package.py` files.

Issue #94: the rez integration shim's bottleneck after the static
parser landed was the serial Python loop of `open()` calls
(~3 s for ~2,600 files on a typical Fortiche resolve, 91% of
`_load_family` wall time). This bench quantifies the win from
batching the reads + parses into a single Rust call.

Run:
  python scripts/bench_batched_parser.py /thierry/rez/pkg [--samples 500]
"""
import argparse
import os
import sys
import time

import pyrer


def walk_package_pys(root):
    """Yield every `package.py` under `root`, skipping rez variant
    subdirs (40-char hex hashes) and dot-directories."""
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [
            d for d in dirnames
            if not d.startswith(".")
            and not d.startswith("_")
            and not (len(d) == 40 and all(c in "0123456789abcdef" for c in d))
        ]
        if "package.py" in filenames:
            yield os.path.join(dirpath, "package.py")


def serial_baseline(paths):
    """Today's shim shape: Python `open` + per-file
    `pyrer.parse_static_package_py(source)` in a loop. The behaviour
    the batched call replaces."""
    results = []
    for p in paths:
        try:
            with open(p, "r", encoding="utf-8") as f:
                source = f.read()
        except OSError:
            results.append(None)
            continue
        results.append(pyrer.parse_static_package_py(source))
    return results


def batched(paths):
    """One Rust call across `len(paths)` files. Rayon thread pool
    handles the parallelism; GIL released for the batch."""
    return pyrer.parse_static_packages_py(paths)


def time_run(fn, paths, repeat=3):
    """Run `fn(paths)` `repeat` times and return the best wall time
    in seconds. Best-of-N filters out CIFS hiccups; the median
    would mask them."""
    best = float("inf")
    for _ in range(repeat):
        t0 = time.perf_counter()
        result = fn(paths)
        elapsed = time.perf_counter() - t0
        best = min(best, elapsed)
    return best, result


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("root", help="rez repo root (e.g. /thierry/rez/pkg)")
    ap.add_argument(
        "--samples",
        type=int,
        default=500,
        help="Number of files to sample from the corpus (default: 500)",
    )
    ap.add_argument(
        "--repeat",
        type=int,
        default=3,
        help="Best-of-N repeats per configuration (default: 3)",
    )
    args = ap.parse_args()

    if not os.path.isdir(args.root):
        print(f"not a directory: {args.root}", file=sys.stderr)
        return 2

    print(f"Sampling up to {args.samples} package.py files from {args.root} ...")
    all_paths = list(walk_package_pys(args.root))
    if len(all_paths) > args.samples:
        step = max(len(all_paths) // args.samples, 1)
        paths = all_paths[::step][: args.samples]
    else:
        paths = all_paths
    print(f"  found {len(all_paths)} files; using {len(paths)} for the bench")
    print()

    # Warm the cache a bit — first read is always I/O-bound on CIFS
    # regardless of method. We're measuring the delta, not the cold
    # number.
    _ = batched(paths)

    print(f"Running each path {args.repeat}x, best wall time:")
    print("-" * 70)

    serial_time, serial_result = time_run(serial_baseline, paths, args.repeat)
    batched_time, batched_result = time_run(batched, paths, args.repeat)

    # Sanity: the two paths must agree on accept count (correctness).
    serial_accepts = sum(1 for r in serial_result if r is not None)
    batched_accepts = sum(1 for r in batched_result if r is not None)
    print(f"  serial open+parse loop:    {serial_time * 1000:8.2f} ms  "
          f"({serial_accepts}/{len(paths)} accepted)")
    print(f"  batched parse_static_pkg:  {batched_time * 1000:8.2f} ms  "
          f"({batched_accepts}/{len(paths)} accepted)")
    print()
    if batched_time > 0:
        speedup = serial_time / batched_time
        savings_ms = (serial_time - batched_time) * 1000
        print(f"  speedup: {speedup:.2f}×")
        print(f"  savings: {savings_ms:.1f} ms over {len(paths)} files "
              f"({savings_ms / len(paths) * 1000:.1f} μs/file)")
        if serial_accepts != batched_accepts:
            print()
            print(f"  WARNING: accept-count mismatch — {serial_accepts} vs "
                  f"{batched_accepts}. Likely an I/O hiccup; rerun.")
    print()
    pool = os.environ.get("RAYON_NUM_THREADS", "(default: logical cores)")
    print(f"Rayon pool size: {pool}")
    print()
    print("Note: serial baseline is what the rez shim does today.")
    print("The batched call replaces it inside `load_family`.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
