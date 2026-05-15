"""Tests for the rich `PackageData` / `ResolvedVariant` API surface
(`pyrer.solve(..., packages=list[PackageData]) -> .resolved_packages`).

The original JSON-string form is also accepted and covered by
``test_solve.py`` and ``test_differential.py``; this file specifically
exercises the new objects so callers do not have to ``json.dumps`` or
unpack tuples.
"""

import pyrer


def _pkg(name, version, requires=None, variants=None):
    return pyrer.PackageData(
        name=name,
        version=version,
        requires=requires or [],
        variants=variants or [],
    )


def test_package_data_construction_and_repr():
    p = pyrer.PackageData(
        name="maya",
        version="2024.0",
        requires=["python-3"],
        variants=[["python-3.10"], ["python-3.11"]],
    )
    assert p.name == "maya"
    assert p.version == "2024.0"
    assert p.requires == ["python-3"]
    assert p.variants == [["python-3.10"], ["python-3.11"]]
    assert "PackageData" in repr(p)
    assert "maya" in repr(p)


def test_package_data_defaults():
    p = pyrer.PackageData(name="foo", version="1.0")
    assert p.requires == []
    assert p.variants == []


def test_solve_with_packagedata_list():
    """The new ergonomic call site: pass a list of PackageData, no JSON."""
    packages = [
        _pkg("app", "1.0.0", requires=["lib-2"]),
        _pkg("lib", "1.0.0"),
        _pkg("lib", "2.0.0"),
    ]
    result = pyrer.solve(["app"], packages)
    assert result.status == "solved", result.failure_description
    assert len(result.resolved_packages) == 2

    # Rich variant access
    by_name = {v.name: v for v in result.resolved_packages}
    assert by_name["app"].version == "1.0.0"
    assert by_name["app"].variant_index is None
    assert by_name["lib"].version == "2.0.0"

    # uri matches rez's "name/version/package.py[idx]" shape
    assert by_name["app"].uri == "app/1.0.0/package.py"

    # `.resolved` (tuple form) is populated alongside for back-compat
    assert set(by_name) == {n for n, _, _ in result.resolved}


def test_resolved_variant_with_variants():
    """A package with variants should report variant_index and the right uri."""
    packages = [
        _pkg("python", "3.10.0"),
        _pkg("python", "3.11.0"),
        _pkg(
            "maya",
            "2024.0",
            variants=[["python-3.10"], ["python-3.11"]],
        ),
    ]
    result = pyrer.solve(["maya-2024"], packages)
    assert result.status == "solved"

    maya = next(v for v in result.resolved_packages if v.name == "maya")
    # rez's default orderer picks the highest python — variant index 1.
    assert maya.variant_index == 1
    assert maya.uri == "maya/2024.0/package.py[1]"

    # The variant's merged requires include the variant-specific dep.
    assert any("python" in r for r in maya.requires)


def test_resolved_variant_requires_merge_base_and_variant():
    """Resolved.requires is the merged (base + variant_index) requirement list."""
    packages = [
        _pkg("python", "3.10.0"),
        _pkg("qt", "5.15.0"),
        _pkg(
            "tool",
            "1.0.0",
            requires=["python-3"],
            variants=[["qt-5"]],
        ),
    ]
    result = pyrer.solve(["tool"], packages)
    assert result.status == "solved"

    tool = next(v for v in result.resolved_packages if v.name == "tool")
    # base ("python-3") + variant ("qt-5") both present in the merged list
    names = {r.split("-")[0] for r in tool.requires}
    assert "python" in names and "qt" in names


def test_solve_wrong_packages_type_raises_typeerror():
    """Passing nonsense as `packages` is the call site's fault — raise.
    PyO3 produces its own TypeError when the argument isn't a list of
    `PackageData`."""
    import pytest

    with pytest.raises(TypeError):
        pyrer.solve(["foo"], 42)
    with pytest.raises(TypeError):
        pyrer.solve(["foo"], "not a list")


def test_solve_duplicate_packagedata_reports_error():
    """Two PackageData entries for the same (name, version) is a data bug."""
    packages = [
        _pkg("foo", "1.0.0"),
        _pkg("foo", "1.0.0"),  # duplicate
    ]
    result = pyrer.solve(["foo"], packages)
    assert result.status == "error"
    assert "duplicate" in result.failure_description.lower()


def test_solveresult_repr_uses_resolved_packages_count():
    result = pyrer.solve(["foo"], [_pkg("foo", "1.0.0")])
    r = repr(result)
    assert "SolveResult" in r and "1 packages" in r
