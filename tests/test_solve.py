"""Basic round-trip test for the pyrer Python module.

Run after ``maturin develop`` inside a virtualenv::

    python tests/test_solve.py
"""

import pyrer


def _packages(repo):
    """Build the ``list[pyrer.PackageData]`` ``pyrer.solve`` expects.

    ``repo`` maps ``name -> {version: requires-list}``; this flattens it.
    """
    return [
        pyrer.PackageData(name=name, version=version, requires=list(requires))
        for name, versions in repo.items()
        for version, requires in versions.items()
    ]


def main():
    # --- 1. Successful resolve with a transitive dependency ---
    pkgs = _packages({"app": {"1.0.0": ["lib-2"]}, "lib": {"1.0.0": [], "2.0.0": []}})
    result = pyrer.solve(["app"], pkgs)
    assert result.status == "solved", (
        f"expected solved, got {result.status}: {result.failure_description}"
    )
    resolved = {(v.name, v.version) for v in result.resolved_packages}
    assert resolved == {("app", "1.0.0"), ("lib", "2.0.0")}, resolved
    assert result.failure_description is None
    assert result.solve_time_ms >= 0
    assert result.num_iterations >= 1
    print("PASS: transitive resolve")

    # --- 2. Highest-version selection ---
    result = pyrer.solve(
        ["foo"], _packages({"foo": {"1.0.0": [], "2.0.0": [], "1.5.0": []}})
    )
    assert result.status == "solved"
    assert [(v.name, v.version, v.variant_index) for v in result.resolved_packages] == [
        ("foo", "2.0.0", None)
    ]
    print("PASS: highest-version selection")

    # --- 3. Failed resolve (conflicting requests) ---
    result = pyrer.solve(
        ["foo-1", "foo-2"], _packages({"foo": {"1.0.0": [], "2.0.0": []}})
    )
    assert result.status == "failed", f"expected failed, got {result.status}"
    assert result.resolved_packages == []
    assert result.failure_description is not None
    print("PASS: conflicting requests fail")

    # --- 4. Missing top-level package -> failed (rez reports this as a
    #        failed resolve, not a crash) ---
    result = pyrer.solve(["nonexistent"], _packages({"foo": {"1.0.0": []}}))
    assert result.status == "failed", f"expected failed, got {result.status}"
    assert result.failure_description is not None
    print("PASS: missing package -> failed")

    # --- 5. Empty request list ---
    result = pyrer.solve([], _packages({"foo": {"1.0.0": []}}))
    assert result.status == "solved"
    assert result.resolved_packages == []
    print("PASS: empty request list")

    # --- 6. Wrong-typed packages argument -> TypeError ---
    try:
        pyrer.solve(["foo"], "not a list")
    except TypeError:
        print("PASS: wrong type -> TypeError")
    else:
        raise AssertionError("expected TypeError for non-list packages")

    # --- 7. SolveResult repr ---
    result = pyrer.solve(["foo"], _packages({"foo": {"1.0.0": []}}))
    r = repr(result)
    assert "SolveResult" in r and "solved" in r
    print("PASS: SolveResult repr")

    print("\nAll tests passed.")


if __name__ == "__main__":
    main()
