//! The value model for a loaded package definition.
//!
//! [`Value`] mirrors the Python objects rez keeps in a package's data dict
//! after `rez.serialise.load_py` / `load_yaml`: literals, containers, and
//! `rez.utils.sourcecode.SourceCode` for `commands()`-style and `@late`
//! functions. Anything that can't be expressed here (classes, modules,
//! arbitrary objects) is reported as [`NeedsPython`](super::NeedsPython)
//! by the loaders instead.

use std::fmt;

/// A Python value as rez stores it in a package's data dict.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    None,
    Bool(bool),
    /// Python `int`. Values outside `i64` make the loaders bail.
    Int(i64),
    Float(f64),
    Str(String),
    List(Vec<Value>),
    Tuple(Vec<Value>),
    /// Python `dict`, insertion-ordered. Keys are unique under Python
    /// equality (`1`, `1.0` and `True` are the same key).
    Dict(Vec<(Value, Value)>),
    /// `rez.utils.sourcecode.SourceCode`.
    Source(SourceCode),
}

/// The state rez pickles for a `SourceCode` (its `__getstate__`), minus
/// `filepath`, which the host knows and sets itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceCode {
    /// Function body text, exactly as `SourceCode._init_from_func` builds it.
    pub source: String,
    /// The `def` name.
    pub funcname: String,
    /// `func._decorators`, in application order (bottom-up in the source).
    pub decorators: Vec<Decorator>,
    /// `True` for `@late` functions, `False` for rex functions
    /// (`commands`, `pre_commands`, ...).
    pub eval_as_function: bool,
}

/// One entry of a function's rez `_decorators` list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decorator {
    /// `"late"` or `"include"`.
    pub name: String,
    /// `nargs` — present for `include` (the module names), absent for `late`.
    pub nargs: Option<Vec<String>>,
}

impl Value {
    /// Python `a == b` for the values this model can hold.
    pub fn py_eq(&self, other: &Value) -> bool {
        use Value::*;
        match (self, other) {
            (None, None) => true,
            (Str(a), Str(b)) => a == b,
            (List(a), List(b)) | (Tuple(a), Tuple(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.py_eq(y))
            }
            (Dict(a), Dict(b)) => {
                a.len() == b.len()
                    && a.iter().all(|(k, v)| {
                        b.iter()
                            .find(|(k2, _)| k.py_eq(k2))
                            .is_some_and(|(_, v2)| v.py_eq(v2))
                    })
            }
            (Source(a), Source(b)) => a.source == b.source,
            _ => match (self.as_number(), other.as_number()) {
                (Some(a), Some(b)) => a.eq(&b),
                _ => false,
            },
        }
    }

    /// Whether the value is hashable in Python (valid as a dict key).
    pub fn is_hashable(&self) -> bool {
        match self {
            Value::List(_) | Value::Dict(_) | Value::Source(_) => false,
            Value::Tuple(items) => items.iter().all(Value::is_hashable),
            _ => true,
        }
    }

    fn as_number(&self) -> Option<Number> {
        match self {
            Value::Bool(b) => Some(Number::Int(*b as i64)),
            Value::Int(i) => Some(Number::Int(*i)),
            Value::Float(f) => Some(Number::Float(*f)),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
enum Number {
    Int(i64),
    Float(f64),
}

impl Number {
    /// Exact int/float comparison, as Python does it (no lossy cast).
    fn eq(&self, other: &Number) -> bool {
        match (*self, *other) {
            (Number::Int(a), Number::Int(b)) => a == b,
            (Number::Float(a), Number::Float(b)) => a == b,
            (Number::Int(i), Number::Float(f)) | (Number::Float(f), Number::Int(i)) => {
                // `f` must be integral and `i` must round-trip exactly.
                f.fract() == 0.0
                    && f >= -9.223_372_036_854_775_808e18
                    && f < 9.223_372_036_854_775_808e18
                    && (f as i64) == i
            }
        }
    }
}

/// Insert into an insertion-ordered Python dict: an existing equal key keeps
/// its position (and its original key object); only the value is replaced.
pub(crate) fn dict_insert(dict: &mut Vec<(Value, Value)>, key: Value, value: Value) {
    if let Some(slot) = dict.iter_mut().find(|(k, _)| k.py_eq(&key)) {
        slot.1 = value;
    } else {
        dict.push((key, value));
    }
}

/// A loaded package definition: the dict rez's `load_from_file` returns,
/// in the same key order.
#[derive(Clone, Debug, PartialEq)]
pub struct PackageDefinition {
    pub data: Vec<(String, Value)>,
}

impl PackageDefinition {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.data.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// Remove a key, returning its value (like `dict.pop(key, None)`).
    pub fn remove(&mut self, key: &str) -> Option<Value> {
        let idx = self.data.iter().position(|(k, _)| k == key)?;
        Some(self.data.remove(idx).1)
    }
}

/// Outcome of loading one definition file. There is no third state: the
/// loader either reproduces rez's result exactly, or says why it can't.
#[derive(Clone, Debug, PartialEq)]
pub enum LoadOutcome {
    Static(PackageDefinition),
    NeedsPython(NeedsPython),
}

/// Why a definition has to go through rez's Python loader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NeedsPython {
    pub reason: NeedsPythonReason,
    /// 1-based line in the definition file, when there is one.
    pub line: Option<u32>,
    /// Human-readable detail for diagnostics; not a stable format.
    pub detail: String,
}

/// Coarse classification of [`NeedsPython`] results, for metrics.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NeedsPythonReason {
    /// Bytes that aren't UTF-8 under the file's declared encoding.
    Encoding,
    /// The file doesn't tokenize as Python (rez would raise).
    Tokenize,
    /// `@early` — evaluated by rez at load time.
    EarlyBinding,
    /// A decorator other than `@late()` / `@include(...)`.
    Decorator,
    /// `import` / `from ... import`.
    Import,
    /// `if`, `for`, `while`, `try`, `match`, ...
    ControlFlow,
    /// A top-level `class`.
    ClassDef,
    /// A top-level `def preprocess`, kept by rez as a raw function.
    Preprocess,
    /// An expression that isn't a literal.
    NonLiteral,
    /// A literal this model can't hold exactly (big int, bytes, f-string,
    /// set, complex, `\N{...}`, lone surrogate, ...).
    UnsupportedLiteral,
    /// A statement form the loader doesn't model.
    UnsupportedStatement,
    /// A `with scope(...)` body the loader doesn't model.
    Scope,
    /// Function source extraction that rez would fail on, or whose result
    /// depends on the Python version running rez.
    SourceExtraction,
    /// YAML the loader doesn't model (tags, anchors, timestamps, ...).
    Yaml,
    /// Valid input on which rez itself would raise; let rez raise it.
    RezError,
}

impl fmt::Display for NeedsPython {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(l) => write!(f, "{:?} at line {}: {}", self.reason, l, self.detail),
            None => write!(f, "{:?}: {}", self.reason, self.detail),
        }
    }
}

pub(crate) fn needs(
    reason: NeedsPythonReason,
    line: Option<u32>,
    detail: impl Into<String>,
) -> NeedsPython {
    NeedsPython {
        reason,
        line,
        detail: detail.into(),
    }
}

/// Python's `str.isspace()` for a single character (`Py_UNICODE_ISSPACE`).
pub(crate) fn py_isspace(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0b}' | '\u{0c}' | '\r' | '\u{1c}'..='\u{1f}' | ' ' | '\u{85}' | '\u{a0}'
            | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}'
            | '\u{205f}' | '\u{3000}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn py_eq_numeric_cross_type() {
        assert!(Value::Int(1).py_eq(&Value::Bool(true)));
        assert!(Value::Int(1).py_eq(&Value::Float(1.0)));
        assert!(!Value::Int(1).py_eq(&Value::Float(1.5)));
        assert!(!Value::Float(f64::NAN).py_eq(&Value::Float(f64::NAN)));
        assert!(!Value::Int(1).py_eq(&Value::Str("1".into())));
        assert!(!Value::List(vec![]).py_eq(&Value::Tuple(vec![])));
    }

    #[test]
    fn dict_insert_keeps_first_key_and_position() {
        let mut d = vec![(Value::Int(1), Value::Str("a".into()))];
        dict_insert(&mut d, Value::Str("x".into()), Value::None);
        dict_insert(&mut d, Value::Bool(true), Value::Str("b".into()));
        assert_eq!(d[0], (Value::Int(1), Value::Str("b".into())));
        assert_eq!(d.len(), 2);
    }
}
