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
    make_shared_cache, FamilyLoader, FamilyMap, PackageRepo, Requirement, ScopeError, Solver,
    SolverStatus, VariantSelectMode,
};
use std::cell::RefCell;
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
    ///
    /// **Faster alternative for raw-data callers:** if you already have the
    /// raw strings (typically from `pkg.resource.data` on a rez `Package`,
    /// which stores `requires` / `variants` as raw `list[str]` /
    /// `list[list[str]]` in the common non-late-bound case), prefer
    /// [`Self::from_strings`]. It skips the per-attribute
    /// `AttributeForwardMeta` lookup, the late-bound wrapping, the
    /// `Requirement` parse, and the `str(Requirement)` round-trip — none
    /// of which produce a different `PackageData` for the common case,
    /// but all of which take real time per package.
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

    /// Build a [`PackageData`] from raw strings, skipping any rez
    /// wrapper-object resolution. Use this when you already have raw
    /// `(name, version, requires, variants)` data — typically pulled from
    /// `pkg.resource.data` on a rez `Package`:
    ///
    /// ```python
    /// data = pkg.resource.data
    /// pd = pyrer.PackageData.from_strings(
    ///     data["name"],
    ///     data["version"],
    ///     data.get("requires"),     # may be None / list[str]
    ///     data.get("variants"),     # may be None / list[list[str]]
    /// )
    /// ```
    ///
    /// Faster than [`Self::from_rez`] on rez-integration hot paths because
    /// it does not trigger rez's `AttributeForwardMeta` per attribute, does
    /// not parse each requirement string into a `Requirement` object, and
    /// does not round-trip each `Requirement` back through `__str__`.
    ///
    /// `requires` and `variants` accept `None` (interpreted as empty),
    /// matching `dict.get(...)` ergonomics.
    ///
    /// Functionally equivalent to the four-arg constructor
    /// `PackageData(name, version, requires, variants)` — both take the
    /// same fast PyO3 extraction path. The classmethod form exists to make
    /// the fast path discoverable alongside [`Self::from_rez`] and to give
    /// the contract a name in callers' code. Closes #88.
    ///
    /// **Caveat — late-bound requirements:** for packages where rez stores
    /// `requires` or `variants` as a `SourceCode` instance (`@early` /
    /// `@late` binding), `pkg.resource.data["requires"]` is *not* a
    /// `list[str]` and this method will raise. Fall back to
    /// [`Self::from_rez`] for those packages — it walks rez's lazy
    /// attribute path which evaluates the source code.
    #[classmethod]
    #[pyo3(signature = (name, version, requires=None, variants=None))]
    fn from_strings(
        _cls: &Bound<'_, PyType>,
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

/// Fold a flat `list[PackageData]` into the `family → version → data` map
/// shape the solver works on. Duplicates (same family + version) raise an
/// `error` result rather than silently shadowing.
fn packages_to_map(packages: Vec<PackageData>) -> Result<HashMap<String, FamilyMap>, String> {
    let mut map: HashMap<String, FamilyMap> = HashMap::new();
    for p in packages {
        let entry = map.entry(p.name.clone()).or_default();
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
    Ok(map)
}

/// Build a [`FamilyLoader`] that calls the given Python callable for each
/// family the solver hasn't yet seen, mirroring issue #86's lazy-discovery
/// shape.
///
/// `load_err` is shared with the caller — if the Python callback raises,
/// the loader stores the error there and returns an empty `Vec`, which the
/// repo memoises as "no such family". The outer `solve()` checks the
/// `RefCell` after the solver finishes and surfaces the captured error as
/// a `"error"`-status `SolveResult`.
///
/// `takes_range` flags whether the callback's signature accepts a second
/// `version_range` parameter (issue #92). When true, the loader passes the
/// current solver range as a rez-style string (or `None` for
/// "unconstrained"); when false, the loader calls with just the name.
/// Detected once via `inspect.signature` in [`solve`].
///
/// Entries whose `name` doesn't match the requested family are dropped
/// defensively — a misbehaving loader can't poison the repo for unrelated
/// families.
fn make_loader(
    callback: Py<PyAny>,
    load_err: Rc<RefCell<Option<String>>>,
    takes_range: bool,
) -> FamilyLoader {
    Box::new(
        move |name: &str,
              hint: Option<&rer_version::VersionRange>|
              -> Vec<(String, rer_resolver::PackageData)> {
            // Already errored on a previous call — short-circuit so we don't
            // pile up errors and don't keep calling a broken callback.
            if load_err.borrow().is_some() {
                return Vec::new();
            }
            let hint_str: Option<String> = hint.map(|r| r.to_string());
            let result: PyResult<Vec<(String, rer_resolver::PackageData)>> =
                Python::with_gil(|py| -> PyResult<_> {
                    let ret = if takes_range {
                        // Pass hint as a rez-style range string via the
                        // `version_range` keyword — works with both
                        // `def f(name, version_range=None)` and
                        // `def f(name, **kwargs)`. `None` → Python
                        // `None`, signalling "unconstrained".
                        let py_hint = match hint_str.as_deref() {
                            Some(s) => s.into_pyobject(py)?.into_any(),
                            None => py.None().into_bound(py),
                        };
                        let kwargs = pyo3::types::PyDict::new(py);
                        kwargs.set_item("version_range", py_hint)?;
                        callback.bind(py).call((name,), Some(&kwargs))?
                    } else {
                        callback.bind(py).call1((name,))?
                    };
                    let pkgs: Vec<PackageData> = ret.extract()?;
                    let mut out: Vec<(String, rer_resolver::PackageData)> =
                        Vec::with_capacity(pkgs.len());
                    let mut seen_versions: std::collections::HashSet<String> =
                        std::collections::HashSet::new();
                    for p in pkgs {
                        if p.name != name {
                            continue;
                        }
                        if !seen_versions.insert(p.version.clone()) {
                            return Err(PyValueError::new_err(format!(
                                "load_family({:?}) returned duplicate version {:?}",
                                name, p.version
                            )));
                        }
                        out.push((
                            p.version,
                            rer_resolver::PackageData {
                                requires: p.requires,
                                variants: p.variants,
                            },
                        ));
                    }
                    Ok(out)
                });
            match result {
                Ok(pairs) => pairs,
                Err(err) => {
                    let msg = Python::with_gil(|py| err.value(py).to_string());
                    *load_err.borrow_mut() = Some(format!("load_family({name:?}) raised: {msg}"));
                    Vec::new()
                }
            }
        },
    )
}

/// Inspect the Python callable's signature and return `true` if it can
/// accept a second `version_range` argument — either as a named parameter
/// or via `**kwargs` / `*args`. False means the legacy 1-arg shape.
///
/// Errs on the side of `false` (legacy) if introspection fails for any
/// reason; the loader then keeps calling with just `name` and existing
/// shims keep working.
fn callback_takes_range(py: Python<'_>, callback: &Py<PyAny>) -> bool {
    let inspect = match py.import("inspect") {
        Ok(m) => m,
        Err(_) => return false,
    };
    let sig = match inspect.getattr("signature").and_then(|f| f.call1((callback,))) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let params = match sig.getattr("parameters") {
        Ok(p) => p,
        Err(_) => return false,
    };
    let len: usize = params.len().unwrap_or(0);
    if len == 0 {
        return false;
    }
    // Walk the parameters' kinds. A second positional parameter, a
    // `version_range` keyword parameter, or any `*args`/`**kwargs`
    // signals support.
    let Ok(items) = params.call_method0("items") else {
        return false;
    };
    let Ok(iter) = items.try_iter() else {
        return false;
    };
    let mut positional_count = 0usize;
    let Ok(inspect_mod) = py.import("inspect") else {
        return false;
    };
    let Ok(parameter_cls) = inspect_mod.getattr("Parameter") else {
        return false;
    };
    let pkw = parameter_cls.getattr("VAR_KEYWORD").ok();
    let pvar = parameter_cls.getattr("VAR_POSITIONAL").ok();
    for item in iter {
        let Ok(item) = item else { continue };
        let Ok(name_val) = item.get_item(0) else {
            continue;
        };
        let Ok(param) = item.get_item(1) else { continue };
        let Ok(name_s) = name_val.extract::<String>() else {
            continue;
        };
        if name_s == "version_range" {
            return true;
        }
        let Ok(kind) = param.getattr("kind") else {
            continue;
        };
        if let Some(p) = &pkw {
            if kind.eq(p).unwrap_or(false) {
                return true;
            }
        }
        if let Some(p) = &pvar {
            if kind.eq(p).unwrap_or(false) {
                return true;
            }
        }
        // Plain positional / positional-or-keyword: counts toward arity.
        positional_count += 1;
        if positional_count >= 2 {
            return true;
        }
    }
    false
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
///   the filesystem itself. Optional if `load_family` is supplied.
/// * `load_family` — Optional `Callable[[str], list[PackageData]]` invoked
///   on demand the first time the solver needs a family that isn't already
///   in `packages`. The result is cached for the lifetime of the solve,
///   so each family is loaded at most once. An empty list means "no such
///   family" and is treated the same as an absent family. See issue #86.
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
        package_requests, packages=None, /,
        *,
        load_family=None,
        variant_select_mode="version_priority",
        filters=None, max_iterations=None,
    )
)]
fn solve(
    package_requests: Vec<String>,
    packages: Option<Vec<PackageData>>,
    load_family: Option<Py<PyAny>>,
    variant_select_mode: &str,
    filters: Option<Vec<(String, String)>>,
    max_iterations: Option<u32>,
) -> PyResult<SolveResult> {
    let _ = (filters, max_iterations);
    let start = Instant::now();

    let mode = parse_variant_select_mode(variant_select_mode)?;

    let initial_map = match packages_to_map(packages.unwrap_or_default()) {
        Ok(map) => map,
        Err(msg) => return Ok(SolveResult::error(msg, start)),
    };

    // Shared error slot for the loader. Populated if the Python callback
    // raises; checked after the solver finishes to surface the failure.
    let load_err: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

    let repo = if let Some(callback) = load_family {
        // One-time signature inspection (issue #92): if the callback can
        // take a `version_range` argument, we pass the current solver
        // range as a rez-style string so the shim can pre-filter.
        // Backward-compatible: callbacks with the 1-arg shape keep
        // working unchanged.
        let takes_range = Python::with_gil(|py| callback_takes_range(py, &callback));
        let lazy = PackageRepo::with_loader(make_loader(
            callback,
            Rc::clone(&load_err),
            takes_range,
        ));
        // Seed the eager set so the loader is never called for families
        // the caller already supplied.
        for (name, fam) in initial_map {
            lazy.insert_family(name, fam);
        }
        lazy
    } else {
        PackageRepo::from_map(initial_map)
    };

    // `Requirement::parse` panics on a syntactically invalid version range;
    // catch that at the FFI boundary and report it as `"error"` rather than
    // letting it surface as a Python `PanicException`.
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let reqs: Vec<Requirement> = package_requests
            .iter()
            .map(|s| Requirement::parse(s))
            .collect();
        let mut solver = Solver::new_with_options(reqs, Rc::new(repo), make_shared_cache(), mode)?;
        solver.solve();
        Ok::<Solver, ScopeError>(solver)
    }));

    // If the loader raised, that's the user-facing error — surface it
    // before whatever fallback status the solver may have produced.
    if let Some(msg) = load_err.borrow_mut().take() {
        return Ok(SolveResult::error(msg, start));
    }

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

// ---------------------------------------------------------------------------
// parse_static_package_py — fast static parser for the rez `package.py` shape
// ---------------------------------------------------------------------------

/// Try to parse `source` as a statically-resolvable rez `package.py`,
/// returning the four solver-relevant fields as a [`PackageData`] —
/// or `None` if the file needs Python evaluation (e.g. `@early` /
/// `@late`-bound `requires`, top-level `if` / `import`, …).
///
/// `None` is *not* an error. It means "the caller should fall back to
/// `pyrer.PackageData.from_rez(pkg)` for this file" — rez's own
/// evaluator will handle the dynamic case.
///
/// Recognises rez's standard patterns: literal `name`, `version`,
/// `requires`, `variants`; ignorable `def commands(...)` /
/// `def pre_commands(...)` function bodies; ignorable
/// `with scope("config") as config: ...` declarative DSL.
///
/// Intended for the rez-integration `load_family` fast path — try
/// this first, fall back to `from_rez(pkg)` on `None`. See the
/// engineering note at
/// `docs/content/docs/engineering/fast-package-py-parser.md`.
#[pyfunction]
fn parse_static_package_py(source: &str) -> Option<PackageData> {
    rer_package::parse_static_package_py(source).map(|info| PackageData {
        name: info.name,
        version: info.version,
        requires: info.requires,
        variants: info.variants,
    })
}

/// Batched variant of [`parse_static_package_py`]: open and parse
/// every path on a Rayon thread pool, returning a list aligned with
/// `paths`. Closes issue #94.
///
/// ```python
/// import pyrer
///
/// paths = [pkg.filepath for pkg in iter_packages(...)]
/// pds = pyrer.parse_static_packages_py(paths)
/// for pd, pkg in zip(pds, pkgs):
///     if pd is None:
///         pd = pyrer.PackageData.from_rez(pkg)  # dynamic / unreadable
///     ...
/// ```
///
/// - **Output is positionally aligned** with the input. A missing
///   file, a parser bail on dynamic content, and an unreadable file
///   all become `None` at the same index.
/// - **No exceptions escape.** Per-file failures map to `None`.
/// - **GIL is released for the batch** via `Python::allow_threads` so
///   other Python threads run during the I/O.
/// - **Pool size** follows Rayon's default (`RAYON_NUM_THREADS` or
///   logical core count). No per-call knob — set the env var to
///   constrain on shared CI hosts.
///
/// Replaces the rez shim's serial Python loop of `open()` calls —
/// ~3 s on a typical 132-package Fortiche resolve, 91% of the
/// `_load_family` budget when all other pyrer wins are stacked.
#[pyfunction]
fn parse_static_packages_py(
    py: Python<'_>,
    paths: Vec<std::path::PathBuf>,
) -> Vec<Option<PackageData>> {
    // Release the GIL while reading + parsing. The closure produces a
    // `Vec<Option<rer_package::PackageInfo>>`; the conversion to the
    // PyO3-managed `PackageData` happens after the GIL is reacquired.
    let infos: Vec<Option<rer_package::PackageInfo>> =
        py.allow_threads(|| rer_package::parse_static_packages_py(&paths));

    infos
        .into_iter()
        .map(|maybe| {
            maybe.map(|info| PackageData {
                name: info.name,
                version: info.version,
                requires: info.requires,
                variants: info.variants,
            })
        })
        .collect()
}

/// The `pyrer` Python module — Rez-compatible package resolver.
#[pymodule]
fn pyrer(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(solve, m)?)?;
    m.add_function(wrap_pyfunction!(parse_static_package_py, m)?)?;
    m.add_function(wrap_pyfunction!(parse_static_packages_py, m)?)?;
    m.add_class::<PackageData>()?;
    m.add_class::<ResolvedVariant>()?;
    m.add_class::<SolveResult>()?;
    Ok(())
}
