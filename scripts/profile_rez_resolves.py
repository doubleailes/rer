"""Profile rez resolves through the patched rez and split wall time into
buckets.

Requires the patched rez (doubleailes/rez claude/rer-experimental plus
scripts/rez-patches/) and pyrer importable. Each request is resolved
with ResolvedContext after package_repository_manager.clear_caches().

Buckets (seconds, summed over requests):

- pyrer.solve_total: the pyrer.solve call, including any Python
  load_family callbacks and, in rer-rust mode, Rust-side loading;
- load_family / iter_packages / static_parse / slow_path:
  the Python load_family path (rer mode, or fallbacks);
- make_variant: post-solve construction of rez Variant objects;
- the remainder of wall_s is the rest of ResolvedContext.

usage::

    profile_rez_resolves.py REPO REQUESTS_JSON N MODE [prof_out]

MODE: py (rez solver), rer (use_rer_solver), rer-rust
(use_rer_solver + use_rer_package_loader). Per-request resolved variants
are written to results_<MODE>.json in the current directory.
"""
import cProfile
import json
import pstats
import sys
import time

repo, reqfile, n, mode = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4]
prof_out = sys.argv[5] if len(sys.argv) > 5 else None

from rez.config import config
config.use_rer_solver = mode in ("rer", "rer-rust")
config.use_rer_package_loader = (mode == "rer-rust")
import pyrer
import rez.rer_solver as rs
from rez.package_repository import package_repository_manager
from rez.resolved_context import ResolvedContext

T = {"load_family": 0.0, "pyrer.solve_total": 0.0, "make_variant": 0.0,
     "iter_packages": 0.0, "static_parse": 0.0, "slow_path": 0.0}

_orig_lf = rs.RerSolver._load_family
def _lf(self, *a, **k):
    t = time.perf_counter(); r = _orig_lf(self, *a, **k); T["load_family"] += time.perf_counter() - t; return r
rs.RerSolver._load_family = _lf

import functools
_orig_solve = pyrer.solve
@functools.wraps(_orig_solve)
def _solve(*a, **k):
    t = time.perf_counter(); r = _orig_solve(*a, **k); T["pyrer.solve_total"] += time.perf_counter() - t; return r
pyrer.solve = _solve

_orig_mv = rs.RerSolver._make_package_variant
def _mv(self, *a, **k):
    t = time.perf_counter(); r = _orig_mv(self, *a, **k); T["make_variant"] += time.perf_counter() - t; return r
rs.RerSolver._make_package_variant = _mv

_orig_ip = rs.iter_packages
def _ip(*a, **k):
    t = time.perf_counter(); r = list(_orig_ip(*a, **k)); T["iter_packages"] += time.perf_counter() - t; return iter(r)
rs.iter_packages = _ip

_orig_batch = pyrer.parse_static_packages_py
def _batch(*a, **k):
    t = time.perf_counter(); r = _orig_batch(*a, **k); T["static_parse"] += time.perf_counter() - t; return r
pyrer.parse_static_packages_py = _batch

_orig_slow = rs._make_package_data_slow
def _slow(*a, **k):
    t = time.perf_counter(); r = _orig_slow(*a, **k); T["slow_path"] += time.perf_counter() - t; return r
rs._make_package_data_slow = _slow

assert rs._supports_load_family(pyrer) or mode=="py"
nloaded = 0
requests = json.load(open(reqfile))[:n]
prof = cProfile.Profile() if prof_out else None
wall = 0.0
ok = 0
results = []
for req in requests:
    package_repository_manager.clear_caches()
    t = time.perf_counter()
    if prof: prof.enable()
    ctx = ResolvedContext(package_requests=req, package_paths=[repo], add_implicit_packages=False)
    if prof: prof.disable()
    wall += time.perf_counter() - t
    ok += bool(ctx.success)
    nloaded += ctx.num_loaded_packages
    results.append(sorted(v.qualified_name + "[%s]" % v.index for v in (ctx.resolved_packages or [])))

print(json.dumps({"mode": mode, "n": n, "solved": ok, "wall_s": round(wall, 3), "num_loaded_packages": nloaded,
                  **{k: round(v, 3) for k, v in T.items()}}))
json.dump(results, open(f"results_{mode}.json", "w"))
if prof:
    prof.dump_stats(prof_out)
