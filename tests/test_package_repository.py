"""``pyrer.PackageRepository`` and ``pyrer.solve(..., package_repository=...)``.

Runs against ``tests/fixtures/repos/{a,b}``. The rez-vs-rer comparison of
the same fixtures is ``scripts/diff_repository.py``; this file checks the
pyrer surface: fallback routing, error reporting, stats.
"""

from __future__ import annotations

import pathlib

import pytest

pyrer = pytest.importorskip("pyrer", reason="pyrer not built (run maturin develop)")

REPOS = pathlib.Path(__file__).resolve().parent / "fixtures" / "repos"
PATHS = [str(REPOS / "a"), str(REPOS / "b")]


@pytest.fixture
def repo():
    return pyrer.PackageRepository(PATHS)


def test_iter_packages_static_family(repo):
    pkgs = repo.iter_packages("plain")
    assert sorted(v for _, v, _, _ in pkgs) == ["1.0", "1.1", "2.0.beta"]
    data = {v: d for _, v, _, d in pkgs}
    assert data["1.0"]["requires"] == ["dep-1"]
    assert data["1.0"]["variants"] == [["p-2"], ["p-3"]]


def test_first_path_wins_and_range(repo):
    pkgs = repo.iter_packages("dep")
    assert sorted(v for _, v, _, _ in pkgs) == ["1.0", "1.5", "2.0"]
    shadowed = [d for _, v, _, d in pkgs if v == "1.5"][0]
    assert "requires" not in shadowed  # repos/a wins over repos/b
    assert [v for _, v, _, _ in repo.iter_packages("dep", "2+")] == ["2.0"]


def test_family_and_package_fallbacks(repo):
    assert isinstance(repo.iter_packages("unver"), pyrer.NeedsPython)
    assert isinstance(repo.iter_packages("combined"), pyrer.NeedsPython)
    mixed = {v: d for _, v, _, d in repo.iter_packages("mixed")}
    assert isinstance(mixed["2.0"], pyrer.NeedsPython)
    assert mixed["2.0"].reason == "Import"


def test_solve_routes_fallbacks_to_python(repo):
    family_calls, package_calls = [], []

    def load_family(name, version_range=None):
        family_calls.append(name)
        return [pyrer.PackageData(name, "1.0", [], [])]

    def load_package(name, version):
        package_calls.append((name, version))
        return pyrer.PackageData(name, version, [], [])

    r = pyrer.solve(["plain-1.1", "mixed", "unver"], package_repository=repo,
                    load_family=load_family, load_package=load_package)
    assert r.status == "solved", r.failure_description
    resolved = {(p.name, p.version) for p in r.resolved_packages}
    assert ("mixed", "2.0") in resolved
    assert ("unver", "1.0") in resolved
    assert family_calls == ["unver"]
    assert package_calls == [("mixed", "2.0")]
    stats = repo.stats()
    assert stats["packages_python"] == {"Import": 1}
    assert stats["families_rust"] >= 3


def test_missing_fallback_is_an_error_not_a_guess(repo):
    r = pyrer.solve(["mixed"], package_repository=repo)
    assert r.status == "error"
    assert "load_package" in r.failure_description


def test_load_package_file():
    d = pyrer.load_package_file(str(REPOS / "a" / "fmtver" / "1.0" / "package.py"))
    assert "format_version" not in d  # popped, like rez's check_format_version
    assert isinstance(pyrer.load_package_file(str(REPOS / "a" / "fmtver" / "2.0" / "package.py")),
                      pyrer.NeedsPython)
