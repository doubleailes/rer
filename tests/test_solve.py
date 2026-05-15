"""Basic round-trip test for the rer Python module.

Run after ``maturin develop`` inside a virtualenv::

    python tests/test_solve.py
"""

import json


def _repo(packages):
    """Build the JSON repository string ``rer.solve`` expects.

    ``packages`` maps ``name -> version -> requires-list``; this wraps each
    into the ``{"requires": [...], "variants": [...]}`` schema.
    """
    return json.dumps(
        {
            name: {
                version: {"requires": list(requires), "variants": []}
                for version, requires in versions.items()
            }
            for name, versions in packages.items()
        }
    )


def main():
    import rer

    # --- 1. Successful resolve with a transitive dependency ---
    repo = _repo({"app": {"1.0.0": ["lib-2"]}, "lib": {"1.0.0": [], "2.0.0": []}})
    result = rer.solve(["app"], repo)
    assert result.status == "solved", (
        f"expected solved, got {result.status}: {result.failure_description}"
    )
    resolved = {(n, v) for n, v, _ in result.resolved}
    assert resolved == {("app", "1.0.0"), ("lib", "2.0.0")}, resolved
    assert result.failure_description is None
    assert result.solve_time_ms >= 0
    assert result.num_iterations >= 1
    print("PASS: transitive resolve")

    # --- 2. Highest-version selection ---
    result = rer.solve(
        ["foo"], _repo({"foo": {"1.0.0": [], "2.0.0": [], "1.5.0": []}})
    )
    assert result.status == "solved"
    assert result.resolved == [("foo", "2.0.0", None)], result.resolved
    print("PASS: highest-version selection")

    # --- 3. Failed resolve (conflicting requests) ---
    result = rer.solve(
        ["foo-1", "foo-2"], _repo({"foo": {"1.0.0": [], "2.0.0": []}})
    )
    assert result.status == "failed", f"expected failed, got {result.status}"
    assert result.resolved == []
    assert result.failure_description is not None
    print("PASS: conflicting requests fail")

    # --- 4. Missing top-level package -> failed (rez reports this as a
    #        failed resolve, not a crash) ---
    result = rer.solve(["nonexistent"], _repo({"foo": {"1.0.0": []}}))
    assert result.status == "failed", f"expected failed, got {result.status}"
    assert result.failure_description is not None
    print("PASS: missing package -> failed")

    # --- 5. Empty request list ---
    result = rer.solve([], _repo({"foo": {"1.0.0": []}}))
    assert result.status == "solved"
    assert result.resolved == []
    print("PASS: empty request list")

    # --- 6. Invalid packages JSON -> error ---
    result = rer.solve(["foo"], "not json")
    assert result.status == "error"
    print("PASS: invalid JSON -> error")

    # --- 7. SolveResult repr ---
    result = rer.solve(["foo"], _repo({"foo": {"1.0.0": []}}))
    r = repr(result)
    assert "SolveResult" in r and "solved" in r
    print("PASS: SolveResult repr")

    print("\nAll tests passed.")


if __name__ == "__main__":
    main()
