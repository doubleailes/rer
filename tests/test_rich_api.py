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
# PackageData.from_strings — raw-string fast path (issue #88)
# ---------------------------------------------------------------------------


def test_from_strings_basic():
    """All four args supplied as raw strings — no wrapper objects involved."""
    pd = pyrer.PackageData.from_strings(
        "maya",
        "2024.0",
        ["python-3"],
        [["python-3.10"], ["python-3.11"]],
    )
    assert pd.name == "maya"
    assert pd.version == "2024.0"
    assert pd.requires == ["python-3"]
    assert pd.variants == [["python-3.10"], ["python-3.11"]]


def test_from_strings_defaults_to_empty():
    """requires=None and variants=None default to empty lists."""
    pd = pyrer.PackageData.from_strings("foo", "1.0")
    assert pd.requires == []
    assert pd.variants == []


def test_from_strings_accepts_none_for_collections():
    """`dict.get("requires")` returns None for a missing key — must accept it."""
    pd = pyrer.PackageData.from_strings("foo", "1.0", None, None)
    assert pd.requires == []
    assert pd.variants == []


def test_from_strings_accepts_tuples_and_iterables():
    """PyO3 extracts Vec<String> from any iterable, not just list."""
    pd = pyrer.PackageData.from_strings(
        "tool",
        "1.0",
        ("python-3", "qt-5"),
        (("linux", "python-3.10"),),
    )
    assert pd.requires == ["python-3", "qt-5"]
    assert pd.variants == [["linux", "python-3.10"]]


def test_from_strings_matches_constructor():
    """`from_strings` must produce the same PackageData as the four-arg
    constructor — same fast PyO3 extraction path, classmethod is just a
    named alias for callers wiring rez's resource.data through pyrer."""
    args = ("maya", "2024.0", ["python-3"], [["python-3.10"], ["python-3.11"]])
    via_classmethod = pyrer.PackageData.from_strings(*args)
    via_constructor = pyrer.PackageData(*args)
    assert via_classmethod.name == via_constructor.name
    assert via_classmethod.version == via_constructor.version
    assert via_classmethod.requires == via_constructor.requires
    assert via_classmethod.variants == via_constructor.variants


def test_from_strings_drives_solve_like_from_rez():
    """End-to-end: a solve fed via from_strings produces the same result as
    one fed via from_rez against an equivalent fake-rez Package."""

    class FakeReq:
        def __init__(self, s):
            self._s = s

        def __str__(self):
            return self._s

    class FakePkg:
        def __init__(self, name, version, requires=None, variants=None):
            self.name = name
            self.version = version
            self.requires = (
                [FakeReq(r) for r in requires] if requires else None
            )
            self.variants = (
                [[FakeReq(r) for r in v] for v in variants] if variants else None
            )

    fakes = [
        FakePkg("app", "1.0.0", requires=["lib-2"]),
        FakePkg("lib", "1.0.0"),
        FakePkg("lib", "2.0.0"),
    ]
    via_from_rez = [pyrer.PackageData.from_rez(p) for p in fakes]

    via_from_strings = [
        pyrer.PackageData.from_strings("app", "1.0.0", ["lib-2"]),
        pyrer.PackageData.from_strings("lib", "1.0.0"),
        pyrer.PackageData.from_strings("lib", "2.0.0"),
    ]

    result_a = pyrer.solve(["app"], via_from_rez)
    result_b = pyrer.solve(["app"], via_from_strings)
    assert result_a.resolved == result_b.resolved
    assert result_a.status == result_b.status == "solved"


def test_from_strings_rejects_non_string_requires():
    """from_strings is the contract-strict fast path — pass it a non-string
    in `requires` and it raises rather than silently stringifying. Use
    `from_rez` (or pre-stringify) for object inputs."""
    import pytest

    class NotAString:
        def __str__(self):
            return "python-3"

    with pytest.raises(TypeError):
        pyrer.PackageData.from_strings("foo", "1.0", [NotAString()])


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


# ---------------------------------------------------------------------------
# load_family — lazy package discovery (issue #86)
# ---------------------------------------------------------------------------


def test_load_family_lazy_only_touched_families():
    """The callback fires only for families the solver actually needs."""
    calls = []

    def loader(name):
        calls.append(name)
        if name == "app":
            return [_pkg("app", "1.0.0", requires=["lib-2"])]
        if name == "lib":
            return [_pkg("lib", "1.0.0"), _pkg("lib", "2.0.0")]
        if name == "unrelated":
            return [_pkg("unrelated", "1.0.0")]
        return []

    result = pyrer.solve(["app"], None, load_family=loader)
    assert result.status == "solved", result.failure_description
    names = {v.name: v.version for v in result.resolved_packages}
    assert names == {"app": "1.0.0", "lib": "2.0.0"}

    assert "app" in calls and "lib" in calls
    assert "unrelated" not in calls, "loader should not be called for unrelated families"


def test_load_family_called_at_most_once_per_family():
    """Diamond dep: app -> lib & util, util -> lib. lib loaded once only."""
    calls = []

    def loader(name):
        calls.append(name)
        if name == "app":
            return [_pkg("app", "1.0.0", requires=["lib", "util"])]
        if name == "util":
            return [_pkg("util", "1.0.0", requires=["lib"])]
        if name == "lib":
            return [_pkg("lib", "1.0.0")]
        return []

    result = pyrer.solve(["app"], None, load_family=loader)
    assert result.status == "solved"
    assert calls.count("lib") == 1, f"loader called {calls.count('lib')}x for 'lib'"


def test_load_family_empty_means_no_such_family():
    """A loader that returns [] for an unknown name produces a failed resolve,
    not a crash."""
    def loader(_name):
        return []

    result = pyrer.solve(["doesnotexist"], None, load_family=loader)
    assert result.status == "failed"
    assert result.failure_description


def test_load_family_works_with_eager_seed():
    """Caller can pre-seed some families; the loader is consulted only
    for ones not in the eager list."""
    calls = []

    def loader(name):
        calls.append(name)
        if name == "lib":
            return [_pkg("lib", "2.0.0")]
        return []

    seed = [_pkg("app", "1.0.0", requires=["lib-2"])]
    result = pyrer.solve(["app"], seed, load_family=loader)
    assert result.status == "solved"
    names = {v.name: v.version for v in result.resolved_packages}
    assert names == {"app": "1.0.0", "lib": "2.0.0"}
    # 'app' came from the eager seed; loader was only asked for 'lib'.
    assert "app" not in calls
    assert calls == ["lib"]


def test_load_family_callback_exception_surfaces_as_error():
    """If the callback raises, the solve returns status='error' with the
    error in the description — never a Python exception out of pyrer."""
    def loader(name):
        raise RuntimeError(f"boom for {name}")

    result = pyrer.solve(["whatever"], None, load_family=loader)
    assert result.status == "error"
    assert "boom for whatever" in (result.failure_description or "")


def test_load_family_filters_mismatched_name():
    """A loader that returns entries for the wrong family name has those
    entries dropped, not silently mixed in."""
    def loader(name):
        if name == "app":
            return [
                _pkg("app", "1.0.0"),
                _pkg("not-app", "1.0.0"),  # bogus — must be dropped
            ]
        return []

    result = pyrer.solve(["app"], None, load_family=loader)
    assert result.status == "solved"
    names = {v.name for v in result.resolved_packages}
    assert names == {"app"}


def test_load_family_duplicate_versions_reports_error():
    """A loader returning two PackageData for the same (family, version) is
    a data bug — surface it rather than silently shadowing."""
    def loader(name):
        if name == "app":
            return [_pkg("app", "1.0.0"), _pkg("app", "1.0.0")]
        return []

    result = pyrer.solve(["app"], None, load_family=loader)
    assert result.status == "error"
    assert "duplicate" in (result.failure_description or "").lower()


# ---------------------------------------------------------------------------
# load_family version_range hint (issue #92)
# ---------------------------------------------------------------------------


def test_load_family_range_hint_passed_for_pinned_request():
    """A `lib-2+<3` request should pass `version_range="2+<3"` to the
    callback so the shim can pre-filter via `iter_packages(range_=...)`."""
    seen = []

    def loader(name, version_range=None):
        seen.append((name, version_range))
        if name == "lib":
            return [_pkg("lib", "2.0.0"), _pkg("lib", "2.5.0")]
        return []

    result = pyrer.solve(["lib-2+<3"], None, load_family=loader)
    assert result.status == "solved"
    assert len(seen) == 1
    name, hint = seen[0]
    assert name == "lib"
    # The exact stringification is rez-syntax-ish; just verify the
    # constraint is communicated.
    assert hint is not None
    assert "2" in hint and "<3" in hint


def test_load_family_legacy_one_arg_callback_still_works():
    """A pre-#92 callback that only accepts `name` must keep working —
    pyrer falls back to the old call shape."""
    seen = []

    def loader(name):  # 1-arg, no version_range
        seen.append(name)
        if name == "lib":
            return [_pkg("lib", "2.0.0")]
        return []

    result = pyrer.solve(["lib"], None, load_family=loader)
    assert result.status == "solved"
    assert seen == ["lib"]


def test_load_family_range_hint_string_format():
    """The hint should be a rez-style range string the shim can pass
    directly to `iter_packages(range_=...)`."""
    captured_hint = []

    def loader(name, version_range=None):
        captured_hint.append((name, version_range))
        return [_pkg(name, "1.5.0"), _pkg(name, "2.0.0")]

    pyrer.solve(["foo-1+<2"], None, load_family=loader)
    assert len(captured_hint) == 1
    name, hint = captured_hint[0]
    assert name == "foo"
    # rez accepts the hint as a string — make sure that's what we pass.
    assert isinstance(hint, str)


def test_load_family_kwargs_callback():
    """A callback using `**kwargs` to accept future args should also work."""
    seen = []

    def loader(name, **kwargs):
        seen.append((name, kwargs.get("version_range")))
        if name == "lib":
            return [_pkg("lib", "1.0.0")]
        return []

    result = pyrer.solve(["lib"], None, load_family=loader)
    assert result.status == "solved"
    assert len(seen) == 1
    # **kwargs accepts the hint argument
    name, hint = seen[0]
    assert name == "lib"


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


# ---------------------------------------------------------------------------
# parse_static_package_py — Rust AST fast-path (RFC: fast-package-py-parser)
# ---------------------------------------------------------------------------


def test_parse_static_package_py_minimal():
    """Smallest static package.py — just name + version."""
    src = 'name = "foo"\nversion = "1.0.0"\n'
    pd = pyrer.parse_static_package_py(src)
    assert pd is not None
    assert pd.name == "foo"
    assert pd.version == "1.0.0"
    assert pd.requires == []
    assert pd.variants == []


def test_parse_static_package_py_full_static():
    """All four solver fields with literal values plus an irrelevant
    `def commands()` block."""
    src = '''
name = "maya"
version = "2024.0"
description = "irrelevant"
requires = ["python-3", "qt-5"]
variants = [["linux", "python-3.10"], ["linux", "python-3.11"]]

def commands():
    env.PYTHONPATH.append("{root}/python")
'''
    pd = pyrer.parse_static_package_py(src)
    assert pd is not None
    assert pd.name == "maya"
    assert pd.version == "2024.0"
    assert pd.requires == ["python-3", "qt-5"]
    assert pd.variants == [["linux", "python-3.10"], ["linux", "python-3.11"]]


def test_parse_static_package_py_with_scope():
    """The dominant Fortiche pattern: solver fields at top level plus a
    `with scope("config")` declarative-DSL block. 35% of Fortiche's
    corpus is this shape."""
    src = '''
# -*- coding: utf-8 -*-
name = "fortichebox"
version = "0.2.0"
requires = ["python-2.7+<3"]

def commands():
    env["FPATH"].append("$SPACE/generic")

with scope("config") as config:
    config.release_packages_path = "/some/path"
'''
    pd = pyrer.parse_static_package_py(src)
    assert pd is not None
    assert pd.name == "fortichebox"
    assert pd.requires == ["python-2.7+<3"]


def test_parse_static_package_py_bails_on_early_requires():
    """@early-bound requires is dynamic — must return None so the caller
    falls back to rez."""
    src = '''
name = "foo"
version = "1.0"

@early()
def requires():
    return ["python-3"]
'''
    assert pyrer.parse_static_package_py(src) is None


def test_parse_static_package_py_bails_on_import():
    src = '''
import os

name = "foo"
version = "1.0"
'''
    assert pyrer.parse_static_package_py(src) is None


def test_parse_static_package_py_bails_on_syntax_error():
    """Unparseable source — return None, not raise."""
    src = 'name = "foo\nversion = "1.0"\n'
    assert pyrer.parse_static_package_py(src) is None


# ---------------------------------------------------------------------------
# parse_static_packages_py — batched / Rayon-parallel (issue #94)
# ---------------------------------------------------------------------------


def test_parse_static_packages_py_empty_input():
    """An empty list returns an empty list — never raises."""
    result = pyrer.parse_static_packages_py([])
    assert result == []


def _write_pkg(tmp_path, name, source):
    """Helper: write `source` to `tmp_path/<name>/package.py` and
    return the file path as a string."""
    d = tmp_path / name
    d.mkdir(parents=True, exist_ok=True)
    pkg = d / "package.py"
    pkg.write_text(source, encoding="utf-8")
    return str(pkg)


def test_parse_static_packages_py_each_file_independent(tmp_path):
    """Static, dynamic, missing — each maps to the right slot in the
    aligned output."""
    static_path = _write_pkg(
        tmp_path,
        "app",
        'name = "app"\nversion = "1.0.0"\nrequires = ["lib-2"]\n',
    )
    dynamic_path = _write_pkg(
        tmp_path,
        "dynamic",
        'import os\nname = "foo"\nversion = "1.0"\n',
    )
    missing_path = str(tmp_path / "phantom" / "package.py")
    static_path_2 = _write_pkg(
        tmp_path,
        "lib",
        'name = "lib"\nversion = "2.0.0"\n',
    )

    result = pyrer.parse_static_packages_py(
        [static_path, dynamic_path, missing_path, static_path_2]
    )
    assert len(result) == 4

    assert result[0] is not None
    assert result[0].name == "app"
    assert result[0].version == "1.0.0"
    assert result[0].requires == ["lib-2"]

    assert result[1] is None    # dynamic — top-level import
    assert result[2] is None    # missing file

    assert result[3] is not None
    assert result[3].name == "lib"


def test_parse_static_packages_py_preserves_order(tmp_path):
    """Rayon's par_iter can complete out of order; the returned list
    must match the input positions exactly."""
    paths = []
    for i in range(20):
        if i % 2 == 0:
            src = f'name = "pkg{i}"\nversion = "1.0"\n'
        else:
            # `if` at module scope → parser bails
            src = f'name = "pkg{i}"\nversion = "1.0"\nif True:\n    pass\n'
        paths.append(_write_pkg(tmp_path, f"pkg{i}", src))

    result = pyrer.parse_static_packages_py(paths)
    assert len(result) == 20
    for i, pd in enumerate(result):
        if i % 2 == 0:
            assert pd is not None, f"index {i}: should be Some"
            assert pd.name == f"pkg{i}"
        else:
            assert pd is None, f"index {i}: should be None (dynamic)"


def test_parse_static_packages_py_accepts_pathlib_paths(tmp_path):
    """The PyO3 binding extracts `PathBuf` — `pathlib.Path` instances
    should work alongside `str`."""
    from pathlib import Path

    p = _write_pkg(tmp_path, "p", 'name = "p"\nversion = "1.0"\n')
    result = pyrer.parse_static_packages_py([Path(p)])
    assert len(result) == 1
    assert result[0] is not None
    assert result[0].name == "p"


def test_parse_static_packages_py_drives_solve(tmp_path):
    """End-to-end: batch-parse a small repo, feed straight into
    `pyrer.solve`."""
    app_path = _write_pkg(
        tmp_path, "app/1.0.0",
        'name = "app"\nversion = "1.0.0"\nrequires = ["lib-2"]\n',
    )
    lib1_path = _write_pkg(
        tmp_path, "lib/1.0.0", 'name = "lib"\nversion = "1.0.0"\n',
    )
    lib2_path = _write_pkg(
        tmp_path, "lib/2.0.0", 'name = "lib"\nversion = "2.0.0"\n',
    )

    result = pyrer.parse_static_packages_py([app_path, lib1_path, lib2_path])
    packages = [pd for pd in result if pd is not None]
    assert len(packages) == 3

    solve = pyrer.solve(["app"], packages)
    assert solve.status == "solved"
    names = {v.name: v.version for v in solve.resolved_packages}
    assert names == {"app": "1.0.0", "lib": "2.0.0"}


def test_parse_static_packages_py_matches_single_file(tmp_path):
    """Sanity: batched and single produce equivalent `PackageData` for
    the same source."""
    src = 'name = "foo"\nversion = "1.0.0"\nrequires = ["bar"]\n'
    single = pyrer.parse_static_package_py(src)

    path = _write_pkg(tmp_path, "foo", src)
    batch = pyrer.parse_static_packages_py([path])

    assert single is not None
    assert len(batch) == 1
    assert batch[0] is not None
    assert single.name == batch[0].name
    assert single.version == batch[0].version
    assert single.requires == batch[0].requires
    assert single.variants == batch[0].variants


def test_parse_static_package_py_roundtrips_through_solve():
    """End-to-end: parse a static package.py → use the PackageData in a
    solve. Should resolve identically to a hand-constructed PackageData."""
    src = '''
name = "app"
version = "1.0.0"
requires = ["lib-2"]
'''
    parsed = pyrer.parse_static_package_py(src)
    constructed = pyrer.PackageData("app", "1.0.0", ["lib-2"])
    libs = [pyrer.PackageData("lib", "1.0.0"), pyrer.PackageData("lib", "2.0.0")]

    result_parsed = pyrer.solve(["app"], [parsed] + libs)
    result_constructed = pyrer.solve(["app"], [constructed] + libs)

    assert result_parsed.status == "solved"
    assert result_constructed.status == "solved"
    assert result_parsed.resolved == result_constructed.resolved


# ---------------------------------------------------------------------------
# package orderer plugin — pyrer.PackageOrderer + register_orderer
# ---------------------------------------------------------------------------


def _three_versions(name):
    """A family `name` with versions 1.0.0 / 2.0.0 / 3.0.0, no deps."""
    return [_pkg(name, "1.0.0"), _pkg(name, "2.0.0"), _pkg(name, "3.0.0")]


def test_package_orderer_default_prefers_highest():
    """No orderer → rez-default order → highest version resolved."""
    result = pyrer.solve(["foo"], _three_versions("foo"))
    assert result.status == "solved"
    assert result.resolved_packages[0].version == "3.0.0"


def test_package_orderer_reverses_preference():
    """An orderer that sorts ascending makes the solver pick the lowest
    version. Selected by registered name."""

    class LowestFirst(pyrer.PackageOrderer):
        name = "test-lowest-first"

        def order(self, family, versions):
            return sorted(versions)  # ascending — lowest most preferred

    pyrer.register_orderer(LowestFirst)
    result = pyrer.solve(
        ["foo"], _three_versions("foo"), package_orderer="test-lowest-first"
    )
    assert result.status == "solved"
    assert result.resolved_packages[0].version == "1.0.0"


def test_package_orderer_accepts_instance_directly():
    """`package_orderer` also takes a PackageOrderer instance, not just a
    registered name."""

    class PinTwo(pyrer.PackageOrderer):
        name = "test-pin-two"

        def order(self, family, versions):
            rest = [v for v in versions if v != "2.0.0"]
            return ["2.0.0", *rest]

    result = pyrer.solve(["foo"], _three_versions("foo"), package_orderer=PinTwo())
    assert result.status == "solved"
    assert result.resolved_packages[0].version == "2.0.0"


def test_package_orderer_unknown_name_raises():
    """Selecting an unregistered name is a caller error."""
    import pytest

    with pytest.raises(ValueError, match="no package orderer registered"):
        pyrer.solve(["foo"], _three_versions("foo"), package_orderer="does-not-exist")


def test_package_orderer_wrong_type_raises():
    import pytest

    with pytest.raises(TypeError, match="package_orderer must be"):
        pyrer.solve(["foo"], _three_versions("foo"), package_orderer=42)


def test_register_orderer_without_name_raises():
    """A PackageOrderer subclass must set a non-empty `name`."""
    import pytest

    class Nameless(pyrer.PackageOrderer):
        def order(self, family, versions):
            return versions

    with pytest.raises(ValueError, match="non-empty"):
        pyrer.register_orderer(Nameless)


def test_register_orderer_rejects_non_orderer():
    import pytest

    with pytest.raises(TypeError, match="PackageOrderer"):
        pyrer.register_orderer(object())


def test_package_orderer_exception_surfaces_as_error():
    """If `order()` raises, the solve returns status='error' — no
    exception escapes pyrer.solve."""

    class Boom(pyrer.PackageOrderer):
        name = "test-boom"

        def order(self, family, versions):
            raise RuntimeError("orderer blew up")

    result = pyrer.solve(["foo"], _three_versions("foo"), package_orderer=Boom())
    assert result.status == "error"
    assert "orderer blew up" in (result.failure_description or "")


def test_package_orderer_partial_output_no_crash():
    """An orderer naming only some versions must not crash — omitted
    versions sink to the bottom (least preferred)."""

    class OnlyOne(pyrer.PackageOrderer):
        name = "test-only-one"

        def order(self, family, versions):
            return ["1.0.0"]  # omits 2.0.0, 3.0.0

    result = pyrer.solve(["foo"], _three_versions("foo"), package_orderer=OnlyOne())
    assert result.status == "solved"
    assert result.resolved_packages[0].version == "1.0.0"


def test_package_orderer_none_is_default():
    """package_orderer=None is identical to omitting it."""
    a = pyrer.solve(["foo"], _three_versions("foo"))
    b = pyrer.solve(["foo"], _three_versions("foo"), package_orderer=None)
    assert a.resolved == b.resolved
