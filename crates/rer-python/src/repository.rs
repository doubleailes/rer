//! `pyrer.PackageRepository` — Rust-side filesystem package loading
//! (`rer_repository`), with per-package / per-family fallback to rez's
//! Python loader. Opt-in: nothing changes unless a `PackageRepository` is
//! passed to `pyrer.solve(..., package_repository=...)`.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use rer_repository::{
    load_definition_file, solver_fields, FamilyLoad, FileFormat, FsSettings, PackageOutcome,
    Repositories,
};
use rer_resolver::rez_solver::FamilyLoader;

use crate::definition::{definition_to_py, needs_python};
use crate::PackageData;

#[derive(Default, Debug)]
pub(crate) struct Stats {
    families_rust: u64,
    families_python: BTreeMap<String, u64>,
    packages_rust: u64,
    packages_python: BTreeMap<String, u64>,
}

/// The configured `packages_path`, loaded by rer.
///
/// Mirrors rez's filesystem repository plugin; construct once per process
/// with the same settings rez uses and keep it (it caches listings and
/// definitions like rez's in-process caches). Call `clear_caches()` where
/// rez's `package_repository_manager.clear_caches()` is called.
#[pyclass(module = "pyrer", frozen)]
pub struct PackageRepository {
    pub(crate) repos: Arc<Repositories>,
    pub(crate) stats: Arc<Mutex<Stats>>,
}

#[pymethods]
impl PackageRepository {
    #[new]
    #[pyo3(signature = (
        paths, *,
        package_filenames = None,
        allow_unversioned_packages = true,
        check_package_definition_files = false,
        disable_pkg_ignore = false,
    ))]
    fn new(
        paths: Vec<PathBuf>,
        package_filenames: Option<Vec<String>>,
        allow_unversioned_packages: bool,
        check_package_definition_files: bool,
        disable_pkg_ignore: bool,
    ) -> Self {
        let settings = FsSettings {
            package_filenames: package_filenames.unwrap_or_else(|| vec!["package".into()]),
            allow_unversioned_packages,
            check_package_definition_files,
            disable_pkg_ignore,
        };
        PackageRepository {
            repos: Arc::new(Repositories::new(&paths, settings)),
            stats: Arc::new(Mutex::new(Stats::default())),
        }
    }

    /// The canonical repository locations (rez's `repo.location`).
    fn locations(&self) -> Vec<String> {
        self.repos
            .repositories()
            .iter()
            .map(|r| r.location().to_string_lossy().into_owned())
            .collect()
    }

    fn clear_caches(&self) {
        self.repos.clear_caches();
    }

    /// Counters since construction: families / packages served by rer vs
    /// handed back to Python (by reason).
    fn stats<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let s = self.stats.lock().unwrap();
        let d = PyDict::new(py);
        d.set_item("families_rust", s.families_rust)?;
        d.set_item("families_python", s.families_python.clone())?;
        d.set_item("packages_rust", s.packages_rust)?;
        d.set_item("packages_python", s.packages_python.clone())?;
        Ok(d)
    }

    /// `rez.packages.iter_packages(name, range_, paths)` with each package's
    /// `resource._data`. Returns a `NeedsPython` when rez must enumerate the
    /// family itself, else a list of `(name, version, filepath, data)` where
    /// `data` is a dict or a `NeedsPython`.
    #[pyo3(signature = (name, version_range = None))]
    fn iter_packages(
        &self,
        py: Python<'_>,
        name: &str,
        version_range: Option<&str>,
    ) -> PyResult<PyObject> {
        let range = version_range.map(rer_version::VersionRange::parse);
        let load = match self.repos.select_family(name, range.as_ref()) {
            Err(why) => FamilyLoad::NeedsPython(why),
            Ok(sel) => {
                let repos = Arc::clone(&self.repos);
                FamilyLoad::Packages(py.allow_threads(move || repos.load_selection(name, sel)))
            }
        };
        match load {
            FamilyLoad::NeedsPython(why) => Ok(Py::new(
                py,
                crate::definition::NeedsPython {
                    reason: "Family".into(),
                    line: None,
                    detail: why,
                },
            )?
            .into_any()),
            FamilyLoad::Packages(pkgs) => {
                let out = pyo3::types::PyList::empty(py);
                for p in pkgs {
                    let data = match &p.outcome {
                        PackageOutcome::Static(def) => definition_to_py(py, def)?,
                        PackageOutcome::NeedsPython(n) => {
                            Py::new(py, needs_python(n.clone()))?.into_any()
                        }
                    };
                    let fp = p.filepath.map(|f| f.to_string_lossy().into_owned());
                    out.append((p.name, p.version, fp, data))?;
                }
                Ok(out.into_any().unbind())
            }
        }
    }
}

/// `FileSystemPackageResource._data` for one definition file: the dict, or a
/// `NeedsPython`. For hosts that hook rez's own `_load`.
#[pyfunction]
#[pyo3(signature = (filepath, format = "py"))]
pub fn load_package_file(py: Python<'_>, filepath: PathBuf, format: &str) -> PyResult<PyObject> {
    let fmt = match format {
        "py" => FileFormat::Py,
        "yaml" => FileFormat::Yaml,
        other => return Err(PyValueError::new_err(format!("unknown format {other:?}"))),
    };
    let dir = filepath.parent().map(PathBuf::from).unwrap_or_default();
    let outcome = py.allow_threads(|| load_definition_file(&filepath, fmt, &dir));
    match outcome {
        PackageOutcome::Static(def) => definition_to_py(py, &def),
        PackageOutcome::NeedsPython(n) => Ok(Py::new(py, needs_python(n))?.into_any()),
    }
}

fn bump(map: &mut BTreeMap<String, u64>, key: String) {
    *map.entry(key).or_default() += 1;
}

/// Build the solver's [`FamilyLoader`] on top of a [`PackageRepository`].
///
/// - Family handed back → `family_fallback` (the existing `load_family`
///   loader built from the Python callback).
/// - Package handed back → `load_package(name, version) -> PackageData`.
pub(crate) fn make_rust_loader(
    repo: &PackageRepository,
    family_fallback: Option<FamilyLoader>,
    load_package: Option<Py<PyAny>>,
    load_err: Rc<RefCell<Option<String>>>,
) -> FamilyLoader {
    let repos = Arc::clone(&repo.repos);
    let stats = Arc::clone(&repo.stats);
    Box::new(move |name, hint| {
        if load_err.borrow().is_some() {
            return Vec::new();
        }
        let load = match repos.select_family(name, hint) {
            Err(why) => FamilyLoad::NeedsPython(why),
            Ok(sel) => Python::with_gil(|py| {
                let repos = Arc::clone(&repos);
                FamilyLoad::Packages(py.allow_threads(move || repos.load_selection(name, sel)))
            }),
        };
        let pkgs = match load {
            FamilyLoad::NeedsPython(why) => {
                bump(
                    &mut stats.lock().unwrap().families_python,
                    family_reason(&why),
                );
                return match &family_fallback {
                    Some(f) => f(name, hint),
                    None => {
                        *load_err.borrow_mut() =
                            Some(format!("family {name:?} needs rez's loader ({why}) but no load_family was given"));
                        Vec::new()
                    }
                };
            }
            FamilyLoad::Packages(p) => p,
        };
        stats.lock().unwrap().families_rust += 1;
        let mut out = Vec::with_capacity(pkgs.len());
        for p in pkgs {
            let fields = match &p.outcome {
                PackageOutcome::Static(def) => {
                    solver_fields(def).map_err(|e| ("SolverFields".to_string(), e))
                }
                PackageOutcome::NeedsPython(n) => {
                    Err((format!("{:?}", n.reason), n.detail.clone()))
                }
            };
            match fields {
                Ok((requires, variants)) => {
                    stats.lock().unwrap().packages_rust += 1;
                    out.push((p.version, rer_resolver::PackageData { requires, variants }));
                }
                Err((reason, _detail)) => {
                    bump(&mut stats.lock().unwrap().packages_python, reason);
                    let Some(cb) = &load_package else {
                        *load_err.borrow_mut() = Some(format!(
                            "package {}-{} needs rez's loader but no load_package was given",
                            p.name, p.version
                        ));
                        return Vec::new();
                    };
                    let res: PyResult<PackageData> =
                        Python::with_gil(|py| cb.bind(py).call1((&p.name, &p.version))?.extract());
                    match res {
                        Ok(pd) => out.push((
                            p.version,
                            rer_resolver::PackageData {
                                requires: pd.requires,
                                variants: pd.variants,
                            },
                        )),
                        Err(err) => {
                            let msg = Python::with_gil(|py| err.value(py).to_string());
                            *load_err.borrow_mut() = Some(format!(
                                "load_package({:?}, {:?}) raised: {msg}",
                                p.name, p.version
                            ));
                            return Vec::new();
                        }
                    }
                }
            }
        }
        out
    })
}

fn family_reason(why: &str) -> String {
    // Coarse buckets for the stats: the first words of the diagnostic.
    why.split(|c: char| c == '"' || c == '{')
        .next()
        .unwrap_or(why)
        .trim()
        .to_string()
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PackageRepository>()?;
    m.add_function(wrap_pyfunction!(load_package_file, m)?)?;
    Ok(())
}
