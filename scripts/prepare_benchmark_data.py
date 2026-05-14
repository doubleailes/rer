#!/usr/bin/env python3
"""Extract rez's bundled benchmark dataset into JSON fixtures for rer's tests.

Requires **no rez installation**: the benchmark ``package.py`` files contain
only module-level literal assignments, which are read here with the stdlib
``ast`` module (no code execution).

Inputs (from the ``rez`` git submodule):
  - rez/src/rez/data/benchmarking/packages.tar.gz
  - rez/src/rez/data/benchmarking/requests.json
  - rez/metrics/benchmarking/artifacts/<latest>/resolves.json  (rez ground truth)

Outputs (gitignored -- generated in CI and locally on demand):
  - data_set/benchmark_packages.json   {name: {version: {requires, variants}}}
  - data_set/benchmark_requests.json   copy of requests.json
  - data_set/benchmark_expected.json   [{request, status, resolved_packages, resolve_time}]

Usage:
  python scripts/prepare_benchmark_data.py [--artifact <dir-name>]
"""
import argparse
import ast
import json
import sys
import tarfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
BENCH_DIR = REPO_ROOT / "rez" / "src" / "rez" / "data" / "benchmarking"
ARTIFACTS_DIR = REPO_ROOT / "rez" / "metrics" / "benchmarking" / "artifacts"
OUT_DIR = REPO_ROOT / "data_set"

# Only these module-level names are read from each package.py.
PACKAGE_FIELDS = {"name", "version", "requires", "variants"}


def parse_package_py(source):
    """Return the package fields from a benchmark ``package.py`` source string.

    The files are pure literal assignments, so we walk the AST and
    ``literal_eval`` the right-hand sides -- nothing is executed.
    """
    fields = {}
    for node in ast.parse(source).body:
        if not isinstance(node, ast.Assign) or len(node.targets) != 1:
            continue
        target = node.targets[0]
        if not isinstance(target, ast.Name) or target.id not in PACKAGE_FIELDS:
            continue
        try:
            fields[target.id] = ast.literal_eval(node.value)
        except (ValueError, SyntaxError):
            pass
    return fields


def build_packages(tar_path):
    """Parse every ``package.py`` in the tarball into the rer package schema."""
    packages = {}
    skipped = 0
    with tarfile.open(tar_path, "r:gz") as tar:
        for member in tar:
            if not member.isfile() or not member.name.endswith("package.py"):
                continue
            handle = tar.extractfile(member)
            if handle is None:
                continue
            fields = parse_package_py(handle.read().decode("utf-8"))
            name, version = fields.get("name"), fields.get("version")
            if name is None or version is None:
                skipped += 1
                continue
            requires = [str(r) for r in fields.get("requires", [])]
            variants = [[str(v) for v in variant]
                        for variant in fields.get("variants", [])]
            packages.setdefault(str(name), {})[str(version)] = {
                "requires": requires,
                "variants": variants,
            }
    return packages, skipped


def latest_artifact():
    """Return the most recent benchmark artifact directory (lexically last)."""
    dirs = sorted(p for p in ARTIFACTS_DIR.iterdir() if p.is_dir())
    if not dirs:
        sys.exit(f"no benchmark artifacts found under {ARTIFACTS_DIR}")
    return dirs[-1]


def build_expected(resolves_path):
    """Trim rez's recorded ``resolves.json`` to the fields the tests need."""
    with open(resolves_path) as handle:
        resolves = json.load(handle)
    return [
        {
            "request": entry["request"],
            "status": entry["status"],
            "resolved_packages": entry.get("resolved_packages"),
            "resolve_time": entry.get("resolve_time"),
        }
        for entry in resolves
    ]


def main():
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "--artifact",
        help="benchmark artifact directory name to use as ground truth "
        "(default: most recent under rez/metrics/benchmarking/artifacts)",
    )
    args = parser.parse_args()

    packages_tar = BENCH_DIR / "packages.tar.gz"
    requests_json = BENCH_DIR / "requests.json"
    for required in (packages_tar, requests_json):
        if not required.exists():
            sys.exit(
                f"missing {required}\n"
                "is the `rez` submodule checked out? run: "
                "git submodule update --init"
            )

    artifact = ARTIFACTS_DIR / args.artifact if args.artifact else latest_artifact()
    resolves_json = artifact / "resolves.json"
    if not resolves_json.exists():
        sys.exit(f"missing {resolves_json}")

    OUT_DIR.mkdir(parents=True, exist_ok=True)

    packages, skipped = build_packages(packages_tar)
    versions = sum(len(v) for v in packages.values())
    print(f"parsed {len(packages)} package families / {versions} versions"
          f" ({skipped} skipped)")

    with open(requests_json) as handle:
        requests = json.load(handle)
    print(f"loaded {len(requests)} resolve requests")

    expected = build_expected(resolves_json)
    print(f"loaded {len(expected)} expected resolves from {artifact.name}")

    for filename, data in (
        ("benchmark_packages.json", packages),
        ("benchmark_requests.json", requests),
        ("benchmark_expected.json", expected),
    ):
        path = OUT_DIR / filename
        with open(path, "w") as handle:
            json.dump(data, handle, sort_keys=True)
        print(f"wrote {path.relative_to(REPO_ROOT)}")


if __name__ == "__main__":
    main()
