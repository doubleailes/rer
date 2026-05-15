"""Differential test: ``pyrer`` vs recorded expectations.

Runs the rer (Rust) solver — rez's own phase-based algorithm, ported — on the
cases in ``tests/differential_cases.json`` and checks each against its
recorded expected outcome.

This exercises the Python bridge (``pyrer.solve``) end to end. The
authoritative head-to-head against rez itself is the Rust integration test
``test_rez_benchmark`` (rez's bundled 188-case benchmark, bit-exact 1:1).

Build the module first (``maturin develop`` inside a venv), then::

    pytest tests/test_differential.py -v

If ``pyrer`` is not installed the whole module is skipped.
"""

from __future__ import annotations

import json
import pathlib

import pytest

pyrer = pytest.importorskip(
    "pyrer", reason="pyrer not built (run maturin develop)"
)

TESTS_DIR = pathlib.Path(__file__).resolve().parent
CASES_PATH = TESTS_DIR / "differential_cases.json"


def _load_cases() -> list[dict]:
    with open(CASES_PATH) as fh:
        return json.load(fh)["cases"]


ALL_CASES = _load_cases()


def _repo_json(packages: dict) -> str:
    """Convert a case's ``{name: {version: [deps]}}`` to the repository JSON
    ``pyrer.solve`` expects: ``{name: {version: {requires, variants}}}``."""
    return json.dumps(
        {
            name: {
                version: {"requires": list(deps), "variants": []}
                for version, deps in versions.items()
            }
            for name, versions in packages.items()
        }
    )


def _resolved_set(result) -> set[tuple[str, str]]:
    return {(name, version) for name, version, _idx in result.resolved}


# Categories whose recorded outcomes are rez-unambiguous and authoritative.
#
# `real_deps` is excluded: those baselines were snapshots of rer's *old*
# pubgrub solver, not rez — and rez itself fails several of them (verified
# directly). For those cases we only assert the solver runs cleanly. The
# authoritative head-to-head against rez is the Rust integration test
# `test_rez_benchmark` (rez's bundled 188-case benchmark, bit-exact 1:1).
_STRICT_CATEGORIES = frozenset(
    {"leaf", "multi_leaf", "exact_version", "conflict", "missing"}
)


@pytest.mark.parametrize("case", ALL_CASES, ids=lambda c: c["id"])
def test_rer_solver(case: dict):
    """The Rust solver runs cleanly and matches each case's recorded outcome."""
    result = pyrer.solve(case["requests"], _repo_json(case.get("packages", {})))

    # "error" means malformed input or a panic — always a real bug.
    assert result.status in ("solved", "failed"), (
        f"[{case['id']}] solver errored: {result.failure_description}"
    )

    # `real_deps` baselines are not authoritative — clean-run check only.
    if case["category"] not in _STRICT_CATEGORIES:
        return

    expected = case["expected_status"]
    assert result.status == expected, (
        f"[{case['id']}] expected status '{expected}', got '{result.status}'"
    )

    # When the case pins an exact resolution, the solved set must match it.
    if result.status == "solved" and case.get("expected_resolved"):
        resolved = _resolved_set(result)
        expected_set = {(pair[0], pair[1]) for pair in case["expected_resolved"]}
        assert resolved == expected_set, (
            f"[{case['id']}] resolved mismatch: "
            f"extra={resolved - expected_set}, missing={expected_set - resolved}"
        )


def test_case_count():
    """The fixture file must contain a reasonable number of cases."""
    assert len(ALL_CASES) >= 50, f"expected >= 50 cases, got {len(ALL_CASES)}"
