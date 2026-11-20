"""The ``version_range`` hint pyrer passes to ``load_family`` must parse in
rez and mean the same thing.

Regression for rez benchmark request #84: a requirement spelled
``moment-7-1.1.0`` combined with ``!moment==7.1.1.0`` produced the hint
``7-1.1.0+<7.1.1.0|...``. rez treats ``7-1.1.0`` and ``7.1.1.0`` as the same
version (separators are cosmetic), so it rejected the hint ("Invalid
bound") and the resolve failed. rer now compares versions by tokens only,
like rez. The randomised version of this check is
``scripts/check_range_roundtrip.py``.
"""

from __future__ import annotations

import pytest

pyrer = pytest.importorskip("pyrer", reason="pyrer not built (run maturin develop)")
rez_version = pytest.importorskip("rez.version", reason="rez not importable")

PACKAGES = {
    "web": {"1": [["moment-7-1.1.0"], []]},
    "moment": {v: [[], []] for v in ["7.0.3.0", "7.1.1.0", "7.1.1.0.vrayFix", "7.1.1.1"]},
}


def test_hints_parse_in_rez_and_resolve_matches():
    hints = []

    def load_family(name, version_range=None):
        hints.append((name, version_range))
        return [pyrer.PackageData(name, v, req, var) for v, (req, var) in PACKAGES.get(name, {}).items()]

    r = pyrer.solve(["web", "!moment==7.1.1.0"], load_family=load_family)
    for name, hint in hints:
        if hint is not None:
            rez_version.VersionRange(hint)  # must not raise
    assert r.status == "solved", r.failure_description
    assert {(p.name, p.version) for p in r.resolved_packages} == {("web", "1"), ("moment", "7.1.1.0.vrayFix")}


def test_separator_only_differences_are_equal_versions():
    # "moment-7-1.1.0" admits exactly what "moment-7.1.1.0" admits.
    def resolve(req):
        r = pyrer.solve(["web"], load_family=lambda name, version_range=None: [
            pyrer.PackageData(name, v, [req] if name == "web" else [], [])
            for v in (["1"] if name == "web" else ["7.1.1.0", "7-1-1-0x", "7.1.1.0.a"])])
        return sorted(p.version for p in r.resolved_packages)

    assert resolve("moment-7-1.1.0") == resolve("moment-7.1.1.0")
