"""Basic round-trip test for the rer_solver Python module.

Run after ``maturin develop`` inside a virtualenv::

    python tests/test_solve.py
"""

import os
import sys


def main():
    import rer_solver

    fixtures = os.path.join(
        os.path.dirname(__file__),
        "..",
        "crates",
        "rer-resolver",
        "tests",
        "fixtures",
        "packages",
    )
    fixtures = os.path.abspath(fixtures)

    # --- 1. Successful resolve ---
    result = rer_solver.solve(["many"], [fixtures])
    assert result.status == "solved", f"expected solved, got {result.status}"
    assert len(result.resolved) == 1, f"expected 1 package, got {len(result.resolved)}"
    name, version, variant_index = result.resolved[0]
    assert name == "many", f"expected 'many', got {name}"
    assert version == "1.2.0", f"expected '1.2.0', got {version}"
    assert variant_index == 0, f"expected variant 0, got {variant_index}"
    assert result.failure_description is None
    assert result.solve_time_ms >= 0
    print("PASS: successful resolve")

    # --- 2. Failed resolve (unknown package) ---
    result = rer_solver.solve(["nonexistent-pkg"], [fixtures])
    assert result.status == "failed", f"expected failed, got {result.status}"
    assert result.resolved == []
    assert result.failure_description is not None
    print("PASS: failed resolve returns status='failed'")

    # --- 3. Empty request list ---
    result = rer_solver.solve([], [fixtures])
    assert result.status == "solved", f"expected solved, got {result.status}"
    assert result.resolved == []
    print("PASS: empty request list")

    # --- 4. SolveResult repr ---
    result = rer_solver.solve(["many"], [fixtures])
    r = repr(result)
    assert "SolveResult" in r
    assert "solved" in r
    print("PASS: SolveResult repr")

    print("\nAll tests passed.")


if __name__ == "__main__":
    main()
