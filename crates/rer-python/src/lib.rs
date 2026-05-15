//! `rer` — the Python bridge to rer's rez-faithful package solver.
//!
//! Exposes [`solve`], which runs [`rer_resolver::rez_solver::Solver`] (the
//! Rust port of rez's own phase-based solver) against an in-memory package
//! repository passed in from Python as JSON.

use pyo3::prelude::*;
use rer_resolver::rez_solver::{PackageRepo, Requirement, ScopeError, Solver, SolverStatus};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;
use std::time::Instant;

/// Result of a package resolve operation.
///
/// Returned by [`solve`]. `status` is `"solved"`, `"failed"` (a genuine
/// resolve conflict), or `"error"` (bad input — malformed JSON, an
/// unparseable request, a missing top-level package).
#[pyclass]
#[derive(Clone)]
pub struct SolveResult {
    /// `"solved"`, `"failed"`, or `"error"`.
    #[pyo3(get)]
    pub status: String,
    /// Resolved packages as `(name, version, variant_index)` tuples. The
    /// variant index is `None` for a package that defines no variants.
    #[pyo3(get)]
    pub resolved: Vec<(String, String, Option<usize>)>,
    /// Human-readable failure or error description, if any.
    #[pyo3(get)]
    pub failure_description: Option<String>,
    /// Wall-clock solve time in milliseconds.
    #[pyo3(get)]
    pub solve_time_ms: f64,
    /// Number of solve steps the solver performed.
    #[pyo3(get)]
    pub num_iterations: u32,
}

#[pymethods]
impl SolveResult {
    fn __repr__(&self) -> String {
        format!(
            "SolveResult(status='{}', resolved={} packages, solve_time_ms={:.2})",
            self.status,
            self.resolved.len(),
            self.solve_time_ms
        )
    }
}

impl SolveResult {
    /// A bad-input result (`status = "error"`).
    fn error(message: String, start: Instant) -> Self {
        SolveResult {
            status: "error".to_string(),
            resolved: Vec::new(),
            failure_description: Some(message),
            solve_time_ms: start.elapsed().as_secs_f64() * 1000.0,
            num_iterations: 0,
        }
    }
}

/// Resolve `requests` against the given package repository.
///
/// # Arguments
///
/// * `requests` — Rez-style requirement strings, e.g. `["python-3", "maya-2024"]`.
/// * `packages` — The package repository as a JSON object mapping
///   `name -> version -> {"requires": [...], "variants": [[...]]}`. This is
///   the data the host (rez) has already loaded; rer does not read the
///   filesystem itself.
/// * `filters` — Optional `(filter_type, pattern)` tuples (reserved, ignored).
/// * `max_iterations` — Optional iteration cap (reserved, ignored).
///
/// # Returns
///
/// A [`SolveResult`]. Failures and bad input are reported via `status`, never
/// as Python exceptions.
#[pyfunction]
#[pyo3(signature = (requests, packages, /, filters=None, max_iterations=None))]
fn solve(
    requests: Vec<String>,
    packages: &str,
    filters: Option<Vec<(String, String)>>,
    max_iterations: Option<u32>,
) -> SolveResult {
    let _ = (filters, max_iterations);
    let start = Instant::now();

    let repo: PackageRepo = match serde_json::from_str(packages) {
        Ok(repo) => repo,
        Err(e) => return SolveResult::error(format!("invalid packages JSON: {e}"), start),
    };

    // `Requirement::parse` panics on a syntactically invalid version range;
    // catch that at the FFI boundary and report it as `"error"` rather than
    // letting it surface as a Python `PanicException`.
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let reqs: Vec<Requirement> = requests.iter().map(|s| Requirement::parse(s)).collect();
        let mut solver = Solver::new(reqs, Rc::new(repo))?;
        solver.solve();
        Ok::<Solver, ScopeError>(solver)
    }));

    let solver = match outcome {
        Ok(Ok(solver)) => solver,
        // A missing top-level package family/version. rez reports this as a
        // failed resolve (not a crash), so we do too.
        Ok(Err(scope_err)) => {
            return SolveResult {
                status: "failed".to_string(),
                resolved: Vec::new(),
                failure_description: Some(scope_err.to_string()),
                solve_time_ms: start.elapsed().as_secs_f64() * 1000.0,
                num_iterations: 0,
            }
        }
        Err(_) => {
            return SolveResult::error(
                "solver panicked (an invalid request string?)".to_string(),
                start,
            )
        }
    };

    let solve_time_ms = start.elapsed().as_secs_f64() * 1000.0;
    let num_iterations = solver.num_solves() as u32;

    match solver.status() {
        SolverStatus::Solved => {
            let resolved = solver
                .resolved_packages()
                .unwrap_or_default()
                .iter()
                .map(|v| (v.name().to_string(), v.version().to_string(), v.index()))
                .collect();
            SolveResult {
                status: "solved".to_string(),
                resolved,
                failure_description: None,
                solve_time_ms,
                num_iterations,
            }
        }
        SolverStatus::Failed => SolveResult {
            status: "failed".to_string(),
            resolved: Vec::new(),
            failure_description: solver.failure_description(),
            solve_time_ms,
            num_iterations,
        },
        other => SolveResult::error(format!("unexpected solver status: {other:?}"), start),
    }
}

/// The `rer` Python module — Rez-compatible package resolver.
#[pymodule]
fn rer(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(solve, m)?)?;
    m.add_class::<SolveResult>()?;
    Ok(())
}
