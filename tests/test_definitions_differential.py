"""Differential test: rez vs rer on full package-definition loading.

Every ``package.py`` / ``package.yaml`` in the corpora below is loaded by
rez (``rez.serialise.load_from_file``) and by ``pyrer.load_package_definition``.
rer may decline a file (``pyrer.NeedsPython``) — the host then uses rez —
but whenever rer returns data it must be exactly rez's data.

Corpora:

- ``tests/fixtures/definitions`` — hand-written edge cases;
- rez's own test data (``rez/src/rez/data/tests``), when the ``rez``
  submodule is checked out;
- a seeded fuzz corpus (``scripts/fuzz_definitions.py``).

Needs both ``pyrer`` (``maturin develop``) and ``rez`` importable; skipped
otherwise.
"""

from __future__ import annotations

import os
import pathlib
import random
import sys

import pytest

pytest.importorskip("pyrer", reason="pyrer not built (run maturin develop)")
pytest.importorskip("rez", reason="rez not importable (pip install ./rez)")

ROOT = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))

from diff_definitions import compare_file, iter_definition_files  # noqa: E402
import fuzz_definitions  # noqa: E402

CORPORA = [ROOT / "tests" / "fixtures" / "definitions", ROOT / "rez" / "src" / "rez" / "data" / "tests"]
FILES = sorted(str(p) for p in iter_definition_files([str(c) for c in CORPORA if c.is_dir()]))
BLOCKERS = ("MISMATCH", "rer_static_rez_error")


@pytest.mark.parametrize("path", FILES, ids=lambda p: os.path.relpath(p, ROOT))
def test_definition_matches_rez(path):
    outcome, reason, detail = compare_file(path)
    assert outcome not in BLOCKERS, detail


def test_fixtures_expected_to_need_python_do():
    # `np_*` fixtures are constructs rer must decline, never load.
    for path in FILES:
        if os.path.basename(os.path.dirname(path)).startswith("np_"):
            outcome, _, _ = compare_file(path)
            assert outcome == "needs_python", path


def test_fixtures_expected_static_are_static():
    # Every other fixture must be loaded by rer (and match, checked above).
    fixtures = ROOT / "tests" / "fixtures" / "definitions"
    for path in FILES:
        if not path.startswith(str(fixtures)):
            continue
        if os.path.basename(os.path.dirname(path)).startswith("np_"):
            continue
        outcome, _, detail = compare_file(path)
        assert outcome == "match", (path, detail)


def test_fuzz(tmp_path):
    r = random.Random(0)
    for i in range(300):
        d = tmp_path / ("p%03d" % i)
        d.mkdir()
        path = d / "package.py"
        path.write_text(fuzz_definitions.rand_package(r), newline="")
        outcome, _, detail = compare_file(str(path))
        assert outcome not in BLOCKERS, (path.read_text(), detail)
