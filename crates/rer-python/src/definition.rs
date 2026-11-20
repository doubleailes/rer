//! Python bindings for full package-definition loading
//! (`rer_package::definition`). Additive: nothing here changes the
//! existing `pyrer` surface.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};
use rer_package::definition::{self as def, LoadOutcome, Value};

/// The state of a `rez.utils.sourcecode.SourceCode`, as its
/// `__getstate__` returns it, minus `filepath` (the caller knows the file).
///
/// Hosts rebuild the real object with
/// `sc = SourceCode.__new__(SourceCode); sc.__setstate__({**state, "filepath": path})`.
#[pyclass(module = "pyrer", frozen, get_all)]
pub struct SourceCodeData {
    pub source: String,
    pub funcname: String,
    pub eval_as_function: bool,
    /// `func._decorators`: list of `{"name": ..., ["nargs": [...]]}` dicts.
    pub decorators: Py<PyList>,
}

#[pymethods]
impl SourceCodeData {
    fn __repr__(&self) -> String {
        format!(
            "SourceCodeData(funcname={:?}, eval_as_function={})",
            self.funcname, self.eval_as_function
        )
    }
}

/// Why a definition must be loaded by rez's Python loader.
#[pyclass(module = "pyrer", frozen, get_all)]
#[derive(Clone)]
pub struct NeedsPython {
    /// Reason category, e.g. `"EarlyBinding"`, `"Import"`.
    pub reason: String,
    pub line: Option<u32>,
    pub detail: String,
}

#[pymethods]
impl NeedsPython {
    fn __repr__(&self) -> String {
        format!(
            "NeedsPython({}, line={:?}, {:?})",
            self.reason, self.line, self.detail
        )
    }
}

pub(crate) fn needs_python(np: def::NeedsPython) -> NeedsPython {
    NeedsPython {
        reason: format!("{:?}", np.reason),
        line: np.line,
        detail: np.detail,
    }
}

pub(crate) fn value_to_py(py: Python<'_>, v: &Value) -> PyResult<PyObject> {
    Ok(match v {
        Value::None => py.None(),
        Value::Bool(b) => b.into_pyobject(py)?.to_owned().into_any().unbind(),
        Value::Int(i) => i.into_pyobject(py)?.into_any().unbind(),
        Value::Float(f) => f.into_pyobject(py)?.into_any().unbind(),
        Value::Str(s) => s.into_pyobject(py)?.into_any().unbind(),
        Value::List(items) => {
            let l = PyList::empty(py);
            for it in items {
                l.append(value_to_py(py, it)?)?;
            }
            l.into_any().unbind()
        }
        Value::Tuple(items) => {
            let objs = items
                .iter()
                .map(|it| value_to_py(py, it))
                .collect::<PyResult<Vec<_>>>()?;
            PyTuple::new(py, objs)?.into_any().unbind()
        }
        Value::Dict(items) => {
            let d = PyDict::new(py);
            for (k, v) in items {
                d.set_item(value_to_py(py, k)?, value_to_py(py, v)?)?;
            }
            d.into_any().unbind()
        }
        Value::Source(sc) => {
            let decorators = PyList::empty(py);
            for dec in &sc.decorators {
                let d = PyDict::new(py);
                if let Some(nargs) = &dec.nargs {
                    d.set_item("nargs", nargs.clone())?;
                }
                d.set_item("name", &dec.name)?;
                decorators.append(d)?;
            }
            Py::new(
                py,
                SourceCodeData {
                    source: sc.source.clone(),
                    funcname: sc.funcname.clone(),
                    eval_as_function: sc.eval_as_function,
                    decorators: decorators.unbind(),
                },
            )?
            .into_any()
        }
    })
}

pub(crate) fn definition_to_py(py: Python<'_>, d: &def::PackageDefinition) -> PyResult<PyObject> {
    let out = PyDict::new(py);
    for (k, v) in &d.data {
        out.set_item(k, value_to_py(py, v)?)?;
    }
    Ok(out.into_any().unbind())
}

pub(crate) fn outcome_to_py(py: Python<'_>, o: &LoadOutcome) -> PyResult<PyObject> {
    match o {
        LoadOutcome::Static(d) => definition_to_py(py, d),
        LoadOutcome::NeedsPython(np) => Ok(Py::new(py, needs_python(np.clone()))?.into_any()),
    }
}

/// Load a package definition from its raw bytes, like
/// `rez.serialise.load_py` / `load_yaml`.
///
/// Returns the data dict rez would produce (`SourceCode` values as
/// [`SourceCodeData`]), or a [`NeedsPython`] when only rez's own loader can
/// produce it. Never guesses; never raises for file content.
///
/// `format` is `"py"` or `"yaml"`.
#[pyfunction]
#[pyo3(signature = (data, format = "py"))]
pub fn load_package_definition(py: Python<'_>, data: &[u8], format: &str) -> PyResult<PyObject> {
    let outcome = match format {
        "py" => py.allow_threads(|| def::load_package_py(data)),
        "yaml" => py.allow_threads(|| def::load_package_yaml(data)),
        other => return Err(PyValueError::new_err(format!("unknown format {other:?}"))),
    };
    outcome_to_py(py, &outcome)
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(load_package_definition, m)?)?;
    m.add_class::<SourceCodeData>()?;
    m.add_class::<NeedsPython>()?;
    Ok(())
}
