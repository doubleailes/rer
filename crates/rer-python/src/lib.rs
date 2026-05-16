//! `pyrer` — the Python bridge to rer's rez-faithful package solver.
//!
//! Exposes [`solve`], which runs [`rer_resolver::rez_solver::Solver`] (the
//! Rust port of rez's own phase-based solver) against an in-memory package
//! repository.
//!
//! The Python-facing API takes the package repository as a
//! `list[PackageData]`, mirroring the shape `rez` carries already-loaded
//! packages in. Resolved packages come back as a list of [`ResolvedVariant`]
//! objects with the attribute surface a `rez.Variant` consumer expects
//! (`name`, `version`, `variant_index`, `requires`, `uri`).

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyType;
use rer_resolver::rez_solver::{
    make_shared_cache, PackageRepo, Requirement, ScopeError, Solver, SolverStatus,
    VariantSelectMode,
};
use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;
use std::time::Instant;

// ---------------------------------------------------------------------------
// PackageData — the input shape
// ---------------------------------------------------------------------------

/// One package version's metadata, as Python sees it.
///
/// Mirrors the fields rez's loaded `Package` exposes that matter to the
/// solver: `name`, `version`, base `requires`, and per-variant `requires`.
/// Construct directly in Python, or build a list from rez's
/// `iter_package_families` and pass it straight to [`solve`] — no
/// `json.dumps` needed.
#[pyclass(module = "pyrer", get_all, set_all)]
#[derive(Clone, Debug)]
pub struct PackageData {
    pub name: String,
    pub version: String,
    pub requires: Vec<String>,
    pub variants: Vec<Vec<String>>,
}

#[pymethods]
impl PackageData {
    /// `PackageData(name, version, requires=None, variants=None)`.
    #[new]
    #[pyo3(signature = (name, version, requires=None, variants=None))]
    fn new(
        name: String,
        version: String,
        requires: Option<Vec<String>>,
        variants: Option<Vec<Vec<String>>>,
    ) -> Self {
        PackageData {
            name,
            version,
            requires: requires.unwrap_or_default(),
            variants: variants.unwrap_or_default(),
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "PackageData(name='{}', version='{}', requires={} reqs, variants={} variants)",
            self.name,
            self.version,
            self.requires.len(),
            self.variants.len()
        )
    }

    /// Build a [`PackageData`] from a rez `Package` (or anything duck-typed
    /// with the same four attributes — `name`, `version`, `requires`,
    /// `variants`).
    ///
    /// Stringifies each requirement (rez's `Requirement` instances render
    /// as the rez requirement string via `__str__`) and the `version` (a
    /// `rez.version.Version` is not a `str` on its own). `None` for either
    /// `requires` or `variants` is treated as empty.
    ///
    /// This is a convenience over the four-field constructor — it lives in
    /// `pyrer` so every integration site doesn't have to write the same
    /// extraction loop. `pyrer` itself does not import rez; this method is
    /// duck-typed and works against any object with the four attributes.
    #[classmethod]
    fn from_rez(_cls: &Bound<'_, PyType>, pkg: &Bound<'_, PyAny>) -> PyResult<Self> {
        let name: String = pkg.getattr("name")?.extract()?;
        let version: String = pkg.getattr("version")?.str()?.extract()?;
        let requires = read_requirement_list(&pkg.getattr("requires")?)?;
        let variants = read_variants_list(&pkg.getattr("variants")?)?;
        Ok(PackageData {
            name,
            version,
            requires,
            variants,
        })
    }
}

/// Pull a flat list of requirement strings from a Python object that is
/// either `None`, a sequence of strings, or a sequence of rez-style
/// `Requirement` objects (anything whose `__str__` is the rez requirement
/// form). Used by `PackageData::from_rez` for both the top-level `requires`
/// and each entry of `variants`.
fn read_requirement_list(obj: &Bound<'_, PyAny>) -> PyResult<Vec<String>> {
    if obj.is_none() {
        return Ok(Vec::new());
    }
    obj.try_iter()?
        .map(|item| item?.str()?.extract::<String>())
        .collect()
}

/// Pull `list[list[str]]` from a Python `variants` attribute that is
/// `None`, an empty list, or a sequence of sequences of `Requirement`s /
/// strings.
fn read_variants_list(obj: &Bound<'_, PyAny>) -> PyResult<Vec<Vec<String>>> {
    if obj.is_none() {
        return Ok(Vec::new());
    }
    obj.try_iter()?
        .map(|inner| read_requirement_list(&inner?))
        .collect()
}

// ---------------------------------------------------------------------------
// ResolvedVariant — the output shape (one per solved package)
// ---------------------------------------------------------------------------

/// One resolved variant, as Python sees it.
///
/// Attribute surface matches `rez.Variant` closely enough that downstream
/// rez consumers (environment build, suite construction, context bundling)
/// do not need a translation step — they can read `name` / `version` /
/// `variant_index` / `requires` / `uri` directly.
///
/// `uri` is formatted in rez's `"name/version/package.py[idx]"` shape — a
/// stable identifier, *not* a filesystem path (rer never reads the FS).
#[pyclass(module = "pyrer", get_all)]
#[derive(Clone, Debug)]
pub struct ResolvedVariant {
    pub name: String,
    pub version: String,
    /// `None` for a package with no variants defined.
    pub variant_index: Option<usize>,
    /// The variant's merged requirement list (base + variant-specific),
    /// rendered as the strings that produced them.
    pub requires: Vec<String>,
    /// rez-style identifier; mirrors `Variant.uri` shape (not a real path).
    pub uri: String,
}

#[pymethods]
impl ResolvedVariant {
    fn __repr__(&self) -> String {
        match self.variant_index {
            Some(idx) => format!("ResolvedVariant({}-{}[{}])", self.name, self.version, idx),
            None => format!("ResolvedVariant({}-{}[])", self.name, self.version),
        }
    }
}

// ---------------------------------------------------------------------------
// SolveResult — the wrapper returned by `solve`
// ---------------------------------------------------------------------------

/// Result of a package resolve operation.
///
/// `status` is `"solved"`, `"failed"` (a genuine resolve conflict), or
/// `"error"` (bad input — malformed JSON, an unparseable request, a missing
/// top-level package).
#[pyclass]
#[derive(Clone)]
pub struct SolveResult {
    /// `"solved"`, `"failed"`, or `"error"`.
    #[pyo3(get)]
    pub status: String,
    /// Resolved variants as rich [`ResolvedVariant`] objects.
    #[pyo3(get)]
    pub resolved_packages: Vec<ResolvedVariant>,
    /// Same resolution as `(name, version, variant_index)` tuples. Kept for
    /// compatibility with the original 0.1.0-rc.5 surface — new code should
    /// prefer `resolved_packages`.
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
    /// Resolved ephemerals as rez-style requirement strings, e.g.
    /// `[".feature-1.5", ".mode-debug"]`. Each entry is the intersected
    /// range of every ephemeral (`.foo`) requirement that participated in
    /// the solve. Empty when no ephemerals were involved, and always empty
    /// for `"failed"` / `"error"` results. Mirrors rez's
    /// `Solver.resolved_ephemerals`.
    #[pyo3(get)]
    pub resolved_ephemerals: Vec<String>,
}

#[pymethods]
impl SolveResult {
    fn __repr__(&self) -> String {
        format!(
            "SolveResult(status='{}', resolved_packages={} packages, solve_time_ms={:.2})",
            self.status,
            self.resolved_packages.len(),
            self.solve_time_ms
        )
    }
}

impl SolveResult {
    /// A bad-input result (`status = "error"`).
    fn error(message: String, start: Instant) -> Self {
        SolveResult {
            status: "error".to_string(),
            resolved_packages: Vec::new(),
            resolved: Vec::new(),
            failure_description: Some(message),
            solve_time_ms: start.elapsed().as_secs_f64() * 1000.0,
            num_iterations: 0,
            resolved_ephemerals: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Repository conversion
// ---------------------------------------------------------------------------

/// Fold a flat `list[PackageData]` into the `family → version → data` repo
/// shape the solver works on. Duplicates (same family + version) raise an
/// `error` result rather than silently shadowing.
fn packages_to_repo(packages: Vec<PackageData>) -> Result<PackageRepo, String> {
    let mut repo: PackageRepo = HashMap::new();
    for p in packages {
        let entry = repo.entry(p.name.clone()).or_default();
        if entry
            .insert(
                p.version.clone(),
                rer_resolver::PackageData {
                    requires: p.requires,
                    variants: p.variants,
                },
            )
            .is_some()
        {
            return Err(format!("duplicate package: {}-{}", p.name, p.version));
        }
    }
    Ok(repo)
}

// ---------------------------------------------------------------------------
// solve
// ---------------------------------------------------------------------------

/// Parse rez's `variant_select_mode` string into the Rust enum. Matches
/// rez's string values (`"version_priority"`, `"intersection_priority"`).
fn parse_variant_select_mode(s: &str) -> PyResult<VariantSelectMode> {
    match s {
        "version_priority" => Ok(VariantSelectMode::VersionPriority),
        "intersection_priority" => Ok(VariantSelectMode::IntersectionPriority),
        other => Err(PyValueError::new_err(format!(
            "variant_select_mode must be 'version_priority' or 'intersection_priority' \
             (got {other:?})"
        ))),
    }
}

/// Resolve `package_requests` against the given package repository.
///
/// # Arguments
///
/// * `package_requests` — rez-style requirement strings, e.g.
///   `["python-3", "maya-2024"]`.
/// * `packages` — a `list[PackageData]`, mirroring rez's already-loaded
///   packages. Construct each entry from a `rez.Package` (via
///   `rez.packages.iter_package_families` etc.) — `pyrer` does not read
///   the filesystem itself.
/// * `variant_select_mode` — either `"version_priority"` (default, rez's
///   default config) or `"intersection_priority"`. Mirrors rez's
///   `config.variant_select_mode`.
/// * `filters` — Optional `(filter_type, pattern)` tuples (reserved, ignored).
/// * `max_iterations` — Optional iteration cap (reserved, ignored).
///
/// # Returns
///
/// A [`SolveResult`]. Failures and bad input are reported via
/// `result.status`, never as a Python exception. Use
/// `result.resolved_packages` for the rich [`ResolvedVariant`] form.
#[pyfunction]
#[pyo3(
    signature = (
        package_requests, packages, /,
        variant_select_mode="version_priority",
        filters=None, max_iterations=None,
    )
)]
fn solve(
    package_requests: Vec<String>,
    packages: Vec<PackageData>,
    variant_select_mode: &str,
    filters: Option<Vec<(String, String)>>,
    max_iterations: Option<u32>,
) -> PyResult<SolveResult> {
    let _ = (filters, max_iterations);
    let start = Instant::now();

    let mode = parse_variant_select_mode(variant_select_mode)?;

    let repo: PackageRepo = match packages_to_repo(packages) {
        Ok(repo) => repo,
        Err(msg) => return Ok(SolveResult::error(msg, start)),
    };

    // `Requirement::parse` panics on a syntactically invalid version range;
    // catch that at the FFI boundary and report it as `"error"` rather than
    // letting it surface as a Python `PanicException`.
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let reqs: Vec<Requirement> = package_requests
            .iter()
            .map(|s| Requirement::parse(s))
            .collect();
        let mut solver =
            Solver::new_with_options(reqs, Rc::new(repo), make_shared_cache(), mode)?;
        solver.solve();
        Ok::<Solver, ScopeError>(solver)
    }));

    let solver = match outcome {
        Ok(Ok(solver)) => solver,
        // A missing top-level package family/version. rez reports this as a
        // failed resolve (not a crash), so we do too.
        Ok(Err(scope_err)) => {
            return Ok(SolveResult {
                status: "failed".to_string(),
                resolved_packages: Vec::new(),
                resolved: Vec::new(),
                failure_description: Some(scope_err.to_string()),
                solve_time_ms: start.elapsed().as_secs_f64() * 1000.0,
                num_iterations: 0,
                resolved_ephemerals: Vec::new(),
            });
        }
        Err(_) => {
            return Ok(SolveResult::error(
                "solver panicked (an invalid request string?)".to_string(),
                start,
            ));
        }
    };

    let solve_time_ms = start.elapsed().as_secs_f64() * 1000.0;
    let num_iterations = solver.num_solves() as u32;

    Ok(match solver.status() {
        SolverStatus::Solved => {
            // Use the borrowing iterator forms — no intermediate Vec, and
            // ephemerals are streamed as `&Requirement` rather than cloned.
            let resolved_packages: Vec<ResolvedVariant> = solver
                .resolved_packages_iter()
                .map(|it| {
                    it.map(|v| ResolvedVariant {
                        name: v.name().to_string(),
                        version: v.version().to_string(),
                        variant_index: v.index(),
                        requires: v
                            .requires()
                            .requirements()
                            .iter()
                            .map(|r| r.to_string())
                            .collect(),
                        uri: match v.index() {
                            Some(idx) => {
                                format!("{}/{}/package.py[{}]", v.name(), v.version(), idx)
                            }
                            None => format!("{}/{}/package.py", v.name(), v.version()),
                        },
                    })
                    .collect()
                })
                .unwrap_or_default();
            let resolved: Vec<(String, String, Option<usize>)> = resolved_packages
                .iter()
                .map(|rv| (rv.name.clone(), rv.version.clone(), rv.variant_index))
                .collect();
            let resolved_ephemerals: Vec<String> = solver
                .resolved_ephemerals_iter()
                .map(|it| it.map(|r| r.to_string()).collect())
                .unwrap_or_default();
            SolveResult {
                status: "solved".to_string(),
                resolved_packages,
                resolved,
                failure_description: None,
                solve_time_ms,
                num_iterations,
                resolved_ephemerals,
            }
        }
        SolverStatus::Failed => SolveResult {
            status: "failed".to_string(),
            resolved_packages: Vec::new(),
            resolved: Vec::new(),
            failure_description: solver.failure_description(),
            solve_time_ms,
            num_iterations,
            resolved_ephemerals: Vec::new(),
        },
        other => SolveResult::error(format!("unexpected solver status: {other:?}"), start),
    })
}

/// The `pyrer` Python module — Rez-compatible package resolver.
#[pymodule]
fn pyrer(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(solve, m)?)?;
    m.add_class::<PackageData>()?;
    m.add_class::<ResolvedVariant>()?;
    m.add_class::<SolveResult>()?;
    Ok(())
}
