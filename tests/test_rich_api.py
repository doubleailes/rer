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


# ---------------------------------------------------------------------------
# resolved_ephemerals — rez's Solver.resolved_ephemerals
# ---------------------------------------------------------------------------


def test_resolved_ephemerals_empty_for_solve_without_ephemerals():
    result = pyrer.solve(["foo"], [_pkg("foo", "1.0.0")])
    assert result.status == "solved"
    assert result.resolved_ephemerals == []


def test_resolved_ephemerals_request_only():
    """A `.foo` requirement on the request should surface intersected."""
    result = pyrer.solve([".feature-1"], [])
    assert result.status == "solved"
    assert result.resolved_ephemerals == [".feature-1"]


def test_resolved_ephemerals_intersection_across_request():
    result = pyrer.solve(
        ["foo", ".feature-1+<3", ".feature-2+"],
        [_pkg("foo", "1.0.0")],
    )
    assert result.status == "solved"
    assert result.resolved_ephemerals == [".feature-2+<3"]


def test_resolved_ephemerals_from_package_requires():
    """An ephemeral contributed by a resolved package's `requires`."""
    result = pyrer.solve(
        ["app"],
        [_pkg("app", "1.0.0", requires=[".mode-debug"])],
    )
    assert result.status == "solved"
    assert result.resolved_ephemerals == [".mode-debug"]


def test_resolved_ephemerals_empty_on_failure():
    result = pyrer.solve([".foo-1", ".foo-2"], [])
    assert result.status == "failed"
    assert result.resolved_ephemerals == []


# ---------------------------------------------------------------------------
# PackageData.from_rez — duck-typed convenience for rez integration
# ---------------------------------------------------------------------------


def test_from_rez_plain_attributes():
    """Anything duck-typed with name/version/requires/variants works."""

    class Pkg:
        name = "maya"
        version = "2024.0"
        requires = ["python-3"]
        variants = [["python-3.10"], ["python-3.11"]]

    pd = pyrer.PackageData.from_rez(Pkg())
    assert pd.name == "maya"
    assert pd.version == "2024.0"
    assert pd.requires == ["python-3"]
    assert pd.variants == [["python-3.10"], ["python-3.11"]]


def test_from_rez_none_collections_become_empty():
    """rez packages with no requires / no variants come through as `None`."""

    class Pkg:
        name = "foo"
        version = "1.0.0"
        requires = None
        variants = None

    pd = pyrer.PackageData.from_rez(Pkg())
    assert pd.requires == []
    assert pd.variants == []


def test_from_rez_stringifies_requirement_objects():
    """rez's `Requirement` objects are not strings — they render via __str__."""

    class FakeReq:
        def __init__(self, s):
            self._s = s

        def __str__(self):
            return self._s

    class Pkg:
        name = "tool"
        version = "1.0.0"
        requires = [FakeReq("python-3"), FakeReq("qt-5")]
        variants = [[FakeReq("linux"), FakeReq("python-3.10")]]

    pd = pyrer.PackageData.from_rez(Pkg())
    assert pd.requires == ["python-3", "qt-5"]
    assert pd.variants == [["linux", "python-3.10"]]


def test_from_rez_stringifies_version():
    """rez's `Version` is not a `str`; from_rez calls `str(version)` for us."""

    class V:
        def __str__(self):
            return "2024.0"

    class Pkg:
        name = "maya"
        version = V()
        requires = None
        variants = None

    pd = pyrer.PackageData.from_rez(Pkg())
    assert pd.version == "2024.0"


def test_from_rez_missing_attribute_raises_attributeerror():
    """A non-Package object missing one of the four attributes is a user bug."""
    import pytest

    class NotAPackage:
        name = "x"
        # no `version`, no `requires`, no `variants`

    with pytest.raises(AttributeError):
        pyrer.PackageData.from_rez(NotAPackage())


# ---------------------------------------------------------------------------
# variant_select_mode — rez's intersection_priority vs version_priority
# ---------------------------------------------------------------------------


def test_variant_select_mode_default_is_version_priority():
    """No-kwarg solve is the same as variant_select_mode='version_priority'."""
    packages = [
        _pkg("python", "3.10.0"),
        _pkg("python", "3.11.0"),
        _pkg("maya", "2024.0", variants=[["python-3.10"], ["python-3.11"]]),
    ]
    a = pyrer.solve(["maya"], packages)
    b = pyrer.solve(["maya"], packages, variant_select_mode="version_priority")
    assert a.resolved == b.resolved


def test_variant_select_mode_intersection_priority_changes_pick():
    """intersection_priority prefers the variant matching MORE of the request,
    even if it pins a lower version.

    Request: [maya, python-3, qt-5]
    Variants of maya-2024:
      [python-3.11]                — matches 1 (python), at higher version
      [python-3.10, qt-5]          — matches 2 (python + qt), at lower python

    version_priority's sort breaks on the python comparison FIRST (both
    variants share `python` in `requested_key`; 3.11 > 3.10) — variant 0
    wins before qt is even considered.

    intersection_priority's primary key is the match count itself (2 > 1) —
    variant 1 wins.
    """
    packages = [
        _pkg("python", "3.10.0"),
        _pkg("python", "3.11.0"),
        _pkg("qt", "5.15.0"),
        _pkg(
            "maya",
            "2024.0",
            variants=[
                ["python-3.11"],
                ["python-3.10", "qt-5"],
            ],
        ),
    ]
    request = ["maya", "python-3", "qt-5"]
    vp = pyrer.solve(request, packages, variant_select_mode="version_priority")
    ip = pyrer.solve(request, packages, variant_select_mode="intersection_priority")
    assert vp.status == "solved"
    assert ip.status == "solved"

    vp_maya = next(v for v in vp.resolved_packages if v.name == "maya")
    ip_maya = next(v for v in ip.resolved_packages if v.name == "maya")
    assert vp_maya.variant_index == 0, "version_priority picks the higher-python variant"
    assert ip_maya.variant_index == 1, "intersection_priority picks the wider-match variant"


def test_variant_select_mode_invalid_raises_valueerror():
    """An unknown mode string is a user error — raise."""
    import pytest

    with pytest.raises(ValueError, match="variant_select_mode"):
        pyrer.solve(["foo"], [_pkg("foo", "1.0.0")], variant_select_mode="nope")


def test_from_rez_used_in_solve():
    """End-to-end: from_rez → solve produces the same result as constructor."""

    class Pkg:
        def __init__(self, name, version, requires=None, variants=None):
            self.name = name
            self.version = version
            self.requires = requires
            self.variants = variants

    fakes = [
        Pkg("app", "1.0.0", requires=["lib-2"]),
        Pkg("lib", "1.0.0"),
        Pkg("lib", "2.0.0"),
    ]
    packages = [pyrer.PackageData.from_rez(p) for p in fakes]
    result = pyrer.solve(["app"], packages)
    assert result.status == "solved"
    names = {v.name for v in result.resolved_packages}
    assert names == {"app", "lib"}
