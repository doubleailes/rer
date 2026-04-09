use pyo3::prelude::*;
use std::path::PathBuf;
use std::time::Instant;

/// Result of a package resolve operation.
///
/// Returned by [`solve()`]. The `status` field indicates whether the resolve
/// succeeded (`"solved"`), failed due to conflicting requirements (`"failed"`),
/// or encountered an unexpected error (`"error"`).
#[pyclass]
#[derive(Clone)]
pub struct SolveResult {
    /// `"solved"`, `"failed"`, or `"error"`.
    #[pyo3(get)]
    pub status: String,
    /// Resolved packages as `(name, version, variant_index)` tuples.
    #[pyo3(get)]
    pub resolved: Vec<(String, String, usize)>,
    /// Human-readable failure or error description, if any.
    #[pyo3(get)]
    pub failure_description: Option<String>,
    /// Wall-clock solve time in milliseconds.
    #[pyo3(get)]
    pub solve_time_ms: f64,
    /// Number of solver iterations (reserved for future use).
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

/// Resolve a set of package requests against the given package paths.
///
/// # Arguments
///
/// * `requests` — Rez-style requirement strings, e.g. `["python-3", "maya-2024"]`.
/// * `package_paths` — Filesystem directories to scan for packages.
/// * `filters` — Optional list of `(filter_type, pattern)` tuples (reserved for future use).
/// * `max_iterations` — Optional iteration cap (reserved for future use).
///
/// # Returns
///
/// A [`SolveResult`] with status `"solved"` on success, or `"failed"` / `"error"`
/// on failure. Errors are reported via the result struct, not as Python exceptions.
#[pyfunction]
#[pyo3(signature = (requests, package_paths, /, filters=None, max_iterations=None))]
fn solve(
    requests: Vec<String>,
    package_paths: Vec<String>,
    filters: Option<Vec<(String, String)>>,
    max_iterations: Option<u32>,
) -> SolveResult {
    let _ = filters;
    let _ = max_iterations;

    let start = Instant::now();
    let paths: Vec<PathBuf> = package_paths.iter().map(PathBuf::from).collect();
    let req_strs: Vec<&str> = requests.iter().map(|s| s.as_str()).collect();

    match rer_resolver::solver(req_strs, paths) {
        Ok(solution) => {
            let resolved: Vec<(String, String, usize)> = solution
                .iter()
                .filter_map(|entry| {
                    // Entries are in the format "name/version/package.py"
                    let parts: Vec<&str> = entry.splitn(3, '/').collect();
                    if parts.len() >= 2 {
                        Some((parts[0].to_string(), parts[1].to_string(), 0))
                    } else {
                        None
                    }
                })
                .collect();
            SolveResult {
                status: "solved".to_string(),
                resolved,
                failure_description: None,
                solve_time_ms: start.elapsed().as_secs_f64() * 1000.0,
                num_iterations: 0,
            }
        }
        Err(e) => SolveResult {
            status: "failed".to_string(),
            resolved: Vec::new(),
            failure_description: Some(format!("{}", e)),
            solve_time_ms: start.elapsed().as_secs_f64() * 1000.0,
            num_iterations: 0,
        },
    }
}

/// The `rer_solver` Python module — Rez-compatible package resolver.
#[pymodule]
fn rer_solver(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(solve, m)?)?;
    m.add_class::<SolveResult>()?;
    Ok(())
}
