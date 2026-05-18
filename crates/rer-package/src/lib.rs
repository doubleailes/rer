//! Fast static parser for the four solver-relevant fields of a rez
//! `package.py`: `name`, `version`, `requires`, `variants`.
//!
//! Bypasses Python entirely. Uses `rustpython-parser` to parse the
//! source into an AST and walks the module's top-level statements,
//! extracting literal values for the four solver fields and bailing
//! on anything that would make the result dynamic.
//!
//! Built to compose with the `load_family` lazy-discovery callback
//! (issue #86): the host's loader can try this first, fall back to
//! rez's `Package` evaluator only when this returns `None`. See the
//! [RFC](../../docs/content/docs/engineering/fast-package-py-parser.md)
//! for design context, scope, and the corpus-survey result that
//! motivated the project (issue #88-ish — survey lives in
//! `scripts/survey_package_py.py`).
//!
//! # Bias toward bailing
//!
//! A false positive (parser accepts a file it shouldn't) produces a
//! different `requires` than rez, which means a different resolve —
//! a silent correctness regression. So this parser is conservative.
//! Anything it doesn't recognise → `None`. The slow path through
//! rez always exists; the fast path is opt-in coverage.

use rustpython_parser::ast::{self, Constant, Expr, Mod, Stmt};
use rustpython_parser::{parse, Mode};

/// The four solver-relevant fields extracted from a `package.py`.
/// Mirrors the shape of `pyrer.PackageData` (post-parse, before any
/// `Requirement` interpretation — these are raw rez requirement
/// strings).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageInfo {
    pub name: String,
    pub version: String,
    pub requires: Vec<String>,
    pub variants: Vec<Vec<String>>,
}

/// Try to parse `source` as a statically-resolvable rez `package.py`.
///
/// Returns `Some(info)` if every top-level statement is recognised
/// and the four solver fields are all literal — `None` otherwise.
/// `None` is *not* an error: it just means "the caller should fall
/// back to rez's evaluator for this file" (e.g. a `@late`-bound
/// `requires`, a top-level `if`, an `import` etc).
///
/// Strict subset of `package.py` syntax accepted:
///
/// - `name = "..."` — string literal required.
/// - `version = "..."` — string literal required.
/// - `requires = ["a", "b", ...]` — optional; list/tuple of string
///   literals when present.
/// - `variants = [["a", ...], ...]` — optional; list/tuple of
///   list/tuple of string literals when present.
/// - `def commands(...)`, `def pre_commands(...)`, etc. — ignored
///   (function bodies don't affect the solve).
/// - `with scope("config") as config: ...` — ignored (rez's
///   declarative DSL for non-solver config).
/// - Module docstring, assignments to non-solver fields, comments
///   — ignored.
///
/// Anything else at module scope causes a bail.
pub fn parse_static_package_py(source: &str) -> Option<PackageInfo> {
    let module = parse(source, Mode::Module, "<package.py>").ok()?;
    let body = match module {
        Mod::Module(m) => m.body,
        _ => return None,
    };

    let mut name: Option<String> = None;
    let mut version: Option<String> = None;
    let mut requires: Option<Vec<String>> = None;
    let mut variants: Option<Vec<Vec<String>>> = None;
    let mut seen_first_stmt = false;

    for stmt in body.iter() {
        // Module docstring — bare string expression as first top-level
        // statement. Ignore it.
        if !seen_first_stmt {
            if let Stmt::Expr(expr) = stmt {
                if str_literal(&expr.value).is_some() {
                    seen_first_stmt = true;
                    continue;
                }
            }
        }
        seen_first_stmt = true;

        match stmt {
            // `import x` / `from x import y`.
            Stmt::Import(_) | Stmt::ImportFrom(_) => return None,

            // `with scope("config") as config: ...` — rez's declarative
            // DSL for non-solver config (paths, release settings). Body
            // only writes attributes of the `as`-name. Walk in
            // defensively to ensure it doesn't touch a solver field.
            Stmt::With(w) if is_scope_with(w) => {
                if with_body_touches_solver_field(&w.body) {
                    return None;
                }
            }
            Stmt::AsyncWith(w) if is_scope_with_async(w) => {
                if with_body_touches_solver_field(&w.body) {
                    return None;
                }
            }
            // Any other top-level control flow.
            Stmt::If(_)
            | Stmt::For(_)
            | Stmt::While(_)
            | Stmt::Try(_)
            | Stmt::With(_)
            | Stmt::AsyncFor(_)
            | Stmt::AsyncWith(_)
            | Stmt::TryStar(_)
            | Stmt::Match(_)
            | Stmt::ClassDef(_)
            | Stmt::Raise(_) => return None,

            // `def commands(...)` etc. Function bodies are
            // solver-irrelevant *unless* the function defines a solver
            // field via `@early` / `@late` — in which case we bail.
            Stmt::FunctionDef(func) => {
                if SOLVER_FIELDS.contains(&func.name.as_str())
                    && function_is_early_or_late(func)
                {
                    return None;
                }
                // A plain `def requires():` without @early is unusual but
                // is functionally dynamic — bail to be safe.
                if SOLVER_FIELDS.contains(&func.name.as_str()) {
                    return None;
                }
                // Other function defs — ignored.
            }
            Stmt::AsyncFunctionDef(func) => {
                if SOLVER_FIELDS.contains(&func.name.as_str()) {
                    return None;
                }
            }

            // The interesting case: assignments to module-level names.
            Stmt::Assign(assign) => {
                for target in assign.targets.iter() {
                    let target_name = match target {
                        Expr::Name(n) => n.id.as_str(),
                        _ => continue, // attribute / subscript / tuple
                    };
                    match target_name {
                        "name" => {
                            name = str_literal(&assign.value);
                            if name.is_none() {
                                return None;
                            }
                        }
                        "version" => {
                            version = str_literal(&assign.value);
                            if version.is_none() {
                                return None;
                            }
                        }
                        "requires" => {
                            requires = list_of_str_literal(&assign.value);
                            if requires.is_none() {
                                return None;
                            }
                        }
                        "variants" => {
                            variants = list_of_list_of_str_literal(&assign.value);
                            if variants.is_none() {
                                return None;
                            }
                        }
                        // Other fields (description, authors, tools, …).
                        // Ignored — they don't affect the solve.
                        _ => {}
                    }
                }
            }

            // `requires: list[str] = [...]` — annotated assignment.
            Stmt::AnnAssign(ann) => {
                let target_name = match ann.target.as_ref() {
                    Expr::Name(n) => n.id.as_str(),
                    _ => continue,
                };
                let value = match ann.value.as_ref() {
                    Some(v) => v,
                    None => {
                        if SOLVER_FIELDS.contains(&target_name) {
                            // Type-only annotation, no value — solver field
                            // unset, treat as missing later.
                            return None;
                        }
                        continue;
                    }
                };
                match target_name {
                    "name" => name = str_literal(value).or_else(|| return_none()),
                    "version" => version = str_literal(value).or_else(|| return_none()),
                    "requires" => requires = list_of_str_literal(value).or_else(|| return_none()),
                    "variants" => {
                        variants = list_of_list_of_str_literal(value).or_else(|| return_none())
                    }
                    _ => {}
                }
                // Catch the "set to non-literal" case for solver fields.
                if SOLVER_FIELDS.contains(&target_name) {
                    let extracted_ok = match target_name {
                        "name" => name.is_some(),
                        "version" => version.is_some(),
                        "requires" => requires.is_some(),
                        "variants" => variants.is_some(),
                        _ => true,
                    };
                    if !extracted_ok {
                        return None;
                    }
                }
            }

            // `name += "foo"` — augmented assignment. Treat as dynamic
            // for solver fields; ignore otherwise.
            Stmt::AugAssign(aug) => {
                if let Expr::Name(n) = aug.target.as_ref() {
                    if SOLVER_FIELDS.contains(&n.id.as_str()) {
                        return None;
                    }
                }
            }

            // Bare expression at module level (e.g. a function call) —
            // could have side effects; bail to be safe.
            Stmt::Expr(_) => return None,

            // Anything else (Delete, Pass, Break, Continue, Global,
            // Nonlocal, Return, TypeAlias …) shouldn't appear at
            // module scope of a real `package.py`. Bail.
            _ => return None,
        }
    }

    // `name` and `version` are required. `requires` and `variants`
    // default to empty.
    Some(PackageInfo {
        name: name?,
        version: version?,
        requires: requires.unwrap_or_default(),
        variants: variants.unwrap_or_default(),
    })
}

// ---------------------------------------------------------------------------
// AST helpers
// ---------------------------------------------------------------------------

const SOLVER_FIELDS: &[&str] = &["name", "version", "requires", "variants"];

/// `None` — used in `or_else` to make the assignment branches read.
fn return_none<T>() -> Option<T> {
    None
}

fn str_literal(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Constant(c) => match &c.value {
            Constant::Str(s) => Some(s.clone()),
            _ => None,
        },
        _ => None,
    }
}

fn list_of_str_literal(expr: &Expr) -> Option<Vec<String>> {
    let elts: &[Expr] = match expr {
        Expr::List(l) => &l.elts,
        Expr::Tuple(t) => &t.elts,
        _ => return None,
    };
    let mut out = Vec::with_capacity(elts.len());
    for e in elts {
        out.push(str_literal(e)?);
    }
    Some(out)
}

fn list_of_list_of_str_literal(expr: &Expr) -> Option<Vec<Vec<String>>> {
    let outer: &[Expr] = match expr {
        Expr::List(l) => &l.elts,
        Expr::Tuple(t) => &t.elts,
        _ => return None,
    };
    let mut out = Vec::with_capacity(outer.len());
    for e in outer {
        out.push(list_of_str_literal(e)?);
    }
    Some(out)
}

fn function_is_early_or_late(func: &ast::StmtFunctionDef) -> bool {
    for deco in &func.decorator_list {
        let name = match deco {
            Expr::Name(n) => n.id.as_str(),
            Expr::Call(c) => match c.func.as_ref() {
                Expr::Name(n) => n.id.as_str(),
                _ => continue,
            },
            _ => continue,
        };
        if name == "early" || name == "late" {
            return true;
        }
    }
    false
}

fn is_scope_with(w: &ast::StmtWith) -> bool {
    !w.items.is_empty() && w.items.iter().all(|item| is_scope_call(&item.context_expr))
}

fn is_scope_with_async(w: &ast::StmtAsyncWith) -> bool {
    !w.items.is_empty() && w.items.iter().all(|item| is_scope_call(&item.context_expr))
}

fn is_scope_call(expr: &Expr) -> bool {
    match expr {
        Expr::Call(c) => matches!(c.func.as_ref(), Expr::Name(n) if n.id.as_str() == "scope"),
        _ => false,
    }
}

/// Walk a `with` body and return `true` if it assigns to any of
/// `name` / `version` / `requires` / `variants` at any depth. Used
/// defensively to keep a pathological `with scope(...)` body from
/// shadowing a solver field.
fn with_body_touches_solver_field(body: &[Stmt]) -> bool {
    for stmt in body {
        match stmt {
            Stmt::Assign(assign) => {
                for target in &assign.targets {
                    if let Expr::Name(n) = target {
                        if SOLVER_FIELDS.contains(&n.id.as_str()) {
                            return true;
                        }
                    }
                }
            }
            Stmt::AnnAssign(ann) => {
                if let Expr::Name(n) = ann.target.as_ref() {
                    if SOLVER_FIELDS.contains(&n.id.as_str()) {
                        return true;
                    }
                }
            }
            Stmt::AugAssign(aug) => {
                if let Expr::Name(n) = aug.target.as_ref() {
                    if SOLVER_FIELDS.contains(&n.id.as_str()) {
                        return true;
                    }
                }
            }
            Stmt::With(w) if !is_scope_with(w) => {
                if with_body_touches_solver_field(&w.body) {
                    return true;
                }
            }
            Stmt::With(w) => {
                if with_body_touches_solver_field(&w.body) {
                    return true;
                }
            }
            Stmt::If(s) => {
                if with_body_touches_solver_field(&s.body)
                    || with_body_touches_solver_field(&s.orelse)
                {
                    return true;
                }
            }
            Stmt::For(s) => {
                if with_body_touches_solver_field(&s.body)
                    || with_body_touches_solver_field(&s.orelse)
                {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Smallest static `package.py`.
    #[test]
    fn parses_minimal_static() {
        let src = r#"
name = "foo"
version = "1.0.0"
"#;
        let info = parse_static_package_py(src).expect("static minimal");
        assert_eq!(info.name, "foo");
        assert_eq!(info.version, "1.0.0");
        assert_eq!(info.requires, Vec::<String>::new());
        assert_eq!(info.variants, Vec::<Vec<String>>::new());
    }

    #[test]
    fn parses_full_static() {
        let src = r#"
name = "maya"
version = "2024.0"
description = "irrelevant"
authors = ["Autodesk"]
requires = ["python-3", "qt-5"]
variants = [["linux", "python-3.10"], ["linux", "python-3.11"]]

def commands():
    env.PYTHONPATH.append("{root}/python")
"#;
        let info = parse_static_package_py(src).expect("full static");
        assert_eq!(info.name, "maya");
        assert_eq!(info.version, "2024.0");
        assert_eq!(info.requires, vec!["python-3", "qt-5"]);
        assert_eq!(
            info.variants,
            vec![
                vec!["linux".to_string(), "python-3.10".to_string()],
                vec!["linux".to_string(), "python-3.11".to_string()]
            ]
        );
    }

    /// The dominant Fortiche-corpus pattern: solver fields at top level
    /// plus a `with scope("config")` block setting non-solver fields.
    /// Accounts for 34.9% of the Fortiche repo per the survey.
    #[test]
    fn parses_with_scope_config() {
        let src = r#"
# -*- coding: utf-8 -*-
name = "fortichebox"
version = "0.2.0"
requires = ["python-2.7+<3"]

def commands():
    env["FPATH"].append("$SPACE/generic")

with scope("config") as config:
    config.release_packages_path = "/some/path"
    config.something_else = 42

timestamp = 1642007300
"#;
        let info = parse_static_package_py(src).expect("with scope is ignorable");
        assert_eq!(info.name, "fortichebox");
        assert_eq!(info.version, "0.2.0");
        assert_eq!(info.requires, vec!["python-2.7+<3"]);
        assert!(info.variants.is_empty());
    }

    #[test]
    fn ignores_module_docstring() {
        let src = r#""""Some docstring."""

name = "foo"
version = "1.0"
"#;
        assert!(parse_static_package_py(src).is_some());
    }

    #[test]
    fn ignores_unknown_top_level_fields() {
        // `description`, `authors`, `tools`, `tests`, `build_command`,
        // `timestamp`, `format_version`, `hashed_variants`, …
        let src = r#"
name = "foo"
version = "1.0"
description = "anything"
authors = ["Alice", "Bob"]
tools = ["foo-cli"]
timestamp = 123456789
format_version = 2
hashed_variants = True
build_command = "cmake ..."
"#;
        assert!(parse_static_package_py(src).is_some());
    }

    // --- Bail cases ----------------------------------------------------

    #[test]
    fn bails_on_at_early_requires() {
        let src = r#"
name = "foo"
version = "1.0"

@early()
def requires():
    return ["python-3"]
"#;
        assert!(parse_static_package_py(src).is_none());
    }

    #[test]
    fn bails_on_at_late_variants() {
        let src = r#"
name = "foo"
version = "1.0"

@late()
def variants():
    return [["a"]]
"#;
        assert!(parse_static_package_py(src).is_none());
    }

    #[test]
    fn bails_on_top_level_if() {
        let src = r#"
name = "foo"
version = "1.0"

import os
if os.getenv("DEV"):
    requires = ["dev-lib"]
else:
    requires = ["prod-lib"]
"#;
        assert!(parse_static_package_py(src).is_none());
    }

    #[test]
    fn bails_on_import() {
        let src = r#"
import sys

name = "foo"
version = "1.0"
"#;
        assert!(parse_static_package_py(src).is_none());
    }

    #[test]
    fn bails_on_from_import() {
        let src = r#"
from sys import platform

name = "foo"
version = "1.0"
"#;
        assert!(parse_static_package_py(src).is_none());
    }

    #[test]
    fn bails_on_classdef() {
        let src = r#"
name = "foo"
version = "1.0"

class Helper:
    pass
"#;
        assert!(parse_static_package_py(src).is_none());
    }

    #[test]
    fn bails_on_non_scope_with() {
        let src = r#"
name = "foo"
version = "1.0"

with open("config.json") as f:
    pass
"#;
        assert!(parse_static_package_py(src).is_none());
    }

    #[test]
    fn bails_on_scope_with_that_touches_solver_field() {
        // Pathological — scope-with that rebinds `name` (in real Python,
        // `name = ...` inside a `with` block IS module-level).
        let src = r#"
name = "foo"
version = "1.0"

with scope("config") as config:
    config.release_path = "/foo"
    name = "rebinding"
"#;
        assert!(parse_static_package_py(src).is_none());
    }

    #[test]
    fn bails_on_non_literal_name() {
        // `name = a + b` — not a literal.
        let src = r#"
prefix = "f"
name = prefix + "oo"
version = "1.0"
"#;
        assert!(parse_static_package_py(src).is_none());
    }

    #[test]
    fn bails_on_function_call_for_requires() {
        let src = r#"
name = "foo"
version = "1.0"
requires = build_requires()
"#;
        assert!(parse_static_package_py(src).is_none());
    }

    #[test]
    fn bails_on_missing_name() {
        let src = r#"
version = "1.0"
"#;
        assert!(parse_static_package_py(src).is_none());
    }

    #[test]
    fn bails_on_missing_version() {
        let src = r#"
name = "foo"
"#;
        assert!(parse_static_package_py(src).is_none());
    }

    #[test]
    fn bails_on_syntax_error() {
        let src = r#"
name = "foo
"#;
        assert!(parse_static_package_py(src).is_none());
    }

    #[test]
    fn requires_can_be_empty_or_absent() {
        let src = r#"
name = "foo"
version = "1.0"
requires = []
"#;
        let info = parse_static_package_py(src).expect("empty requires is fine");
        assert!(info.requires.is_empty());
    }

    #[test]
    fn variants_can_be_a_tuple_of_tuples() {
        // rez accepts list-of-lists as the canonical form; we accept
        // tuples too defensively.
        let src = r#"
name = "foo"
version = "1.0"
variants = (("a",), ("b",))
"#;
        let info = parse_static_package_py(src).expect("tuple variants");
        assert_eq!(
            info.variants,
            vec![vec!["a".to_string()], vec!["b".to_string()]]
        );
    }
}
