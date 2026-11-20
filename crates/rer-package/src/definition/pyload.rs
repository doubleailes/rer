//! `rez.serialise.load_py` for the statically-decidable subset of
//! `package.py`.
//!
//! rez `exec`s the file with six injected globals, keeps the resulting
//! globals (minus the injected ones, builtins and `__`-names), merges the
//! `with scope(...)` dicts, converts rex / `@late` functions to `SourceCode`,
//! evaluates `@early` functions, and drops every other function except
//! `preprocess`. This module reproduces that for files made only of:
//!
//! - assignments of a literal to a plain name;
//! - `def` (undecorated, or `@late()` / `@include("mod", ...)`);
//! - `with scope("name") [as x]:` blocks of literal assignments;
//! - docstring-style string statements and `pass`.
//!
//! Everything else yields [`NeedsPython`]: the host runs rez's own loader.

use super::source::{function_source, SourceError};
use super::tokenize::{tokenize, Kind, Tok, Tokens};
use super::value::{
    dict_insert, needs, Decorator, LoadOutcome, NeedsPython, NeedsPythonReason as R,
    PackageDefinition, SourceCode, Value,
};

/// `rez.package_resources.package_rex_keys`.
const REX_KEYS: &[&str] = &[
    "pre_commands",
    "commands",
    "post_commands",
    "pre_build_commands",
    "pre_test_commands",
];

/// Names rez injects into the exec globals (plus `__builtins__`), which are
/// excluded from the result. Rebinding one of them is refused outright.
const INJECTED: &[&str] = &[
    "scope",
    "early",
    "late",
    "include",
    "ModifyList",
    "InvalidPackageError",
    "__builtins__",
];

const KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield",
];

type Res<T> = Result<T, NeedsPython>;

/// Load a `package.py` from its raw bytes, like `rez.serialise.load_py`.
pub fn load_package_py(bytes: &[u8]) -> LoadOutcome {
    match load(bytes) {
        Ok(def) => LoadOutcome::Static(def),
        Err(np) => LoadOutcome::NeedsPython(np),
    }
}

fn load(bytes: &[u8]) -> Res<PackageDefinition> {
    let text = decode(bytes)?;
    let toks = tokenize(&text).map_err(|e| needs(R::Tokenize, Some(e.row), e.msg))?;
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut m = Module {
        toks: &toks,
        lines: &lines,
        i: 0,
        globals: Vec::new(),
        scopes: Vec::new(),
    };
    m.run()?;
    m.finish()
}

// ---------------------------------------------------------------------------
// Decoding (PEP 263 + universal newlines)
// ---------------------------------------------------------------------------

fn decode(bytes: &[u8]) -> Res<String> {
    let (bytes, had_bom) = match bytes.strip_prefix(b"\xef\xbb\xbf") {
        Some(rest) => (rest, true),
        None => (bytes, false),
    };
    if bytes.contains(&0) {
        return Err(needs(R::Encoding, None, "NUL byte in source"));
    }
    // Coding cookie on line 1 or 2 (line 2 only if line 1 is a comment/blank).
    let mut cookie: Option<String> = None;
    for (idx, line) in bytes.split(|b| *b == b'\n').take(2).enumerate() {
        let line = String::from_utf8_lossy(line);
        let trimmed = line.trim_start_matches([' ', '\t', '\x0c']);
        if !trimmed.starts_with('#') {
            if idx == 0 && !trimmed.trim().is_empty() {
                break;
            }
            continue;
        }
        // `#.*?coding[:=][ \t]*([-\w.]+)` — any occurrence may match.
        for (pos, _) in trimmed.match_indices("coding") {
            let rest = &trimmed[pos + 6..];
            if let Some(rest) = rest.strip_prefix([':', '=']) {
                let name: String = rest
                    .trim_start_matches([' ', '\t'])
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
                    .collect();
                if !name.is_empty() {
                    cookie = Some(name.to_ascii_lowercase().replace('_', "-"));
                    break;
                }
            }
        }
        if cookie.is_some() {
            break;
        }
    }
    let is_utf8 = |c: &str| c == "utf-8" || c == "utf8" || c.starts_with("utf-8-");
    if let Some(c) = &cookie {
        if had_bom && !is_utf8(c) {
            return Err(needs(R::Encoding, None, "BOM with non-utf-8 coding cookie"));
        }
        if !is_utf8(c) {
            let ascii_compatible = matches!(
                c.as_str(),
                "latin-1"
                    | "iso-8859-1"
                    | "iso8859-1"
                    | "latin1"
                    | "ascii"
                    | "us-ascii"
                    | "cp1252"
                    | "iso-8859-15"
                    | "l1"
            );
            if !ascii_compatible || !bytes.is_ascii() {
                return Err(needs(R::Encoding, None, format!("coding cookie {c:?}")));
            }
        }
    }
    let s = std::str::from_utf8(bytes).map_err(|_| needs(R::Encoding, None, "invalid utf-8"))?;
    let mut s = s.replace("\r\n", "\n").replace('\r', "\n");
    if !s.is_empty() && !s.ends_with('\n') {
        s.push('\n');
    }
    Ok(s)
}

// ---------------------------------------------------------------------------
// Module evaluation
// ---------------------------------------------------------------------------

/// A global as rez's exec dict would hold it.
#[derive(Clone, Debug)]
enum Global {
    Val(Value),
    /// A function object, resolved in `finish()` like `process_python_objects`.
    Func(Func),
    /// The `as` target of a `with scope(...)` block (a `_Scope` object).
    ScopeObj,
}

#[derive(Clone, Debug)]
struct Func {
    name: String,
    /// rez `_decorators`, application order.
    decorators: Vec<Decorator>,
    late: bool,
    source: Result<String, SourceError>,
    row: u32,
}

/// `rez.utils.scope.RecursiveAttribute` contents. Values are either raw
/// (as assigned) or nested recursive attributes.
#[derive(Clone, Debug, Default)]
struct RecAttr {
    data: Vec<(Value, ScopeVal)>,
}

#[derive(Clone, Debug)]
enum ScopeVal {
    Raw(Value),
    Attr(RecAttr),
}

impl RecAttr {
    /// `RecursiveAttribute(dict)` — nested dicts become attributes too.
    fn from_dict(d: &[(Value, Value)]) -> RecAttr {
        RecAttr {
            data: d
                .iter()
                .map(|(k, v)| (k.clone(), ScopeVal::wrap(v)))
                .collect(),
        }
    }

    /// `_update`: dict values are wrapped, everything else stored raw.
    fn update(&mut self, items: Vec<(Value, Value)>) {
        for (k, v) in items {
            self.set(k, ScopeVal::wrap(&v));
        }
    }

    fn set(&mut self, key: Value, val: ScopeVal) {
        if let Some(slot) = self.data.iter_mut().find(|(k, _)| k.py_eq(&key)) {
            slot.1 = val;
        } else {
            self.data.push((key, val));
        }
    }

    fn to_dict(&self) -> Value {
        Value::Dict(
            self.data
                .iter()
                .map(|(k, v)| {
                    let v = match v {
                        ScopeVal::Raw(v) => v.clone(),
                        ScopeVal::Attr(a) => a.to_dict(),
                    };
                    (k.clone(), v)
                })
                .collect(),
        )
    }
}

impl ScopeVal {
    fn wrap(v: &Value) -> ScopeVal {
        match v {
            Value::Dict(d) => ScopeVal::Attr(RecAttr::from_dict(d)),
            other => ScopeVal::Raw(other.clone()),
        }
    }
}

struct Module<'t, 's> {
    toks: &'t Tokens<'s>,
    lines: &'t [&'s str],
    i: usize,
    /// The exec globals dict, insertion-ordered.
    globals: Vec<(String, Global)>,
    /// `ScopeContext.scopes`, keyed by single-level path (nesting refused).
    scopes: Vec<(String, RecAttr)>,
}

impl<'t, 's> Module<'t, 's> {
    fn peek(&self) -> &'t Tok {
        &self.toks.toks[self.i]
    }

    fn peek_at(&self, off: usize) -> &'t Tok {
        let idx = (self.i + off).min(self.toks.toks.len() - 1);
        &self.toks.toks[idx]
    }

    fn text(&self, t: &Tok) -> &'s str {
        self.toks.text(t)
    }

    fn bump(&mut self) -> &'t Tok {
        let t = &self.toks.toks[self.i];
        if t.kind != Kind::EndMarker {
            self.i += 1;
        }
        t
    }

    fn is_op(&self, t: &Tok, op: &str) -> bool {
        t.kind == Kind::Op && self.text(t) == op
    }

    fn is_name(&self, t: &Tok, name: &str) -> bool {
        t.kind == Kind::Name && self.text(t) == name
    }

    fn expect_op(&mut self, op: &str) -> Res<()> {
        let t = self.bump();
        if self.is_op(t, op) {
            Ok(())
        } else {
            Err(unsupported(t, format!("expected {op:?}")))
        }
    }

    fn expect_newline(&mut self) -> Res<()> {
        if self.peek().kind == Kind::Comment {
            self.bump();
        }
        let t = self.bump();
        if t.kind == Kind::Newline {
            Ok(())
        } else {
            Err(unsupported(t, "expected end of statement"))
        }
    }

    fn skip_blank(&mut self) {
        while matches!(self.peek().kind, Kind::Nl | Kind::Comment) {
            self.i += 1;
        }
    }

    fn set_global(&mut self, name: &str, g: Global) {
        if let Some(slot) = self.globals.iter_mut().find(|(k, _)| k == name) {
            slot.1 = g;
        } else {
            self.globals.push((name.to_string(), g));
        }
    }

    fn run(&mut self) -> Res<()> {
        loop {
            self.skip_blank();
            let t = self.peek();
            match t.kind {
                Kind::EndMarker => return Ok(()),
                Kind::Indent | Kind::Dedent => {
                    return Err(unsupported(t, "unexpected indentation"));
                }
                _ => self.statement()?,
            }
        }
    }

    fn statement(&mut self) -> Res<()> {
        let t = self.peek();
        let text = self.text(t);
        match t.kind {
            Kind::Op if text == "@" => self.decorated_def(),
            Kind::Str => {
                // Expression statement made of a string literal (docstring).
                self.literal()?;
                self.expect_newline()
            }
            Kind::Name => {
                match text {
                    "def" => self.def(t.row, Vec::new(), false),
                    "with" => self.with_scope(),
                    "pass" => {
                        self.bump();
                        self.expect_newline()
                    }
                    "import" | "from" => Err(needs(R::Import, Some(t.row), "import statement")),
                    "class" => Err(needs(R::ClassDef, Some(t.row), "class definition")),
                    "if" | "for" | "while" | "try" | "match" | "async" | "del" | "global"
                    | "nonlocal" | "raise" | "assert" | "return" | "yield" | "lambda" => Err(
                        needs(R::ControlFlow, Some(t.row), format!("`{text}` statement")),
                    ),
                    _ if self.is_op(self.peek_at(1), "=") => self.assignment(),
                    _ => Err(needs(R::NonLiteral, Some(t.row), "expression statement")),
                }
            }
            _ => Err(unsupported(t, "unrecognised statement")),
        }
    }

    fn check_target(&self, t: &Tok) -> Res<String> {
        let name = self.text(t);
        if t.kind != Kind::Name || KEYWORDS.contains(&name) {
            return Err(unsupported(t, "assignment target"));
        }
        if !name.is_ascii() {
            // Python NFKC-normalises identifiers.
            return Err(needs(
                R::UnsupportedStatement,
                Some(t.row),
                "non-ASCII identifier",
            ));
        }
        if INJECTED.contains(&name) {
            return Err(needs(
                R::UnsupportedStatement,
                Some(t.row),
                format!("rebinds rez global {name:?}"),
            ));
        }
        Ok(name.to_string())
    }

    fn assignment(&mut self) -> Res<()> {
        let t = self.bump();
        let name = self.check_target(t)?;
        self.expect_op("=")?;
        let value = self.literal()?;
        self.expect_newline()?;
        self.set_global(&name, Global::Val(value));
        Ok(())
    }

    fn decorated_def(&mut self) -> Res<()> {
        let start_row = self.peek().row;
        let mut source_order: Vec<Decorator> = Vec::new();
        let mut late = false;
        while self.is_op(self.peek(), "@") {
            let at = self.bump();
            let name_tok = self.bump();
            let name = self.text(name_tok);
            match (name_tok.kind, name) {
                (Kind::Name, "early") => {
                    return Err(needs(R::EarlyBinding, Some(at.row), "@early function"))
                }
                (Kind::Name, "late") => {
                    self.expect_op("(")?;
                    self.expect_op(")")?;
                    late = true;
                    source_order.push(Decorator {
                        name: "late".into(),
                        nargs: None,
                    });
                }
                (Kind::Name, "include") => {
                    self.expect_op("(")?;
                    let mut args = Vec::new();
                    loop {
                        if self.is_op(self.peek(), ")") {
                            break;
                        }
                        let v = self.literal()?;
                        let Value::Str(s) = v else {
                            return Err(needs(
                                R::Decorator,
                                Some(at.row),
                                "non-string @include argument",
                            ));
                        };
                        args.push(s);
                        if self.is_op(self.peek(), ",") {
                            self.bump();
                            continue;
                        }
                        break;
                    }
                    self.expect_op(")")?;
                    if args.is_empty() {
                        return Err(needs(
                            R::RezError,
                            Some(at.row),
                            "@include() without a module",
                        ));
                    }
                    source_order.push(Decorator {
                        name: "include".into(),
                        nargs: Some(args),
                    });
                }
                _ => {
                    return Err(needs(
                        R::Decorator,
                        Some(at.row),
                        format!("decorator @{name}"),
                    ));
                }
            }
            // Decorators can be followed by NL-free comments only.
            self.expect_newline()?;
            self.skip_blank();
        }
        if !self.is_name(self.peek(), "def") {
            return Err(unsupported(self.peek(), "decorator not followed by def"));
        }
        // Decorators apply bottom-up: `_decorators` is in reverse source order.
        source_order.reverse();
        self.def(start_row, source_order, late)
    }

    fn def(&mut self, start_row: u32, decorators: Vec<Decorator>, late: bool) -> Res<()> {
        let def_tok = self.bump(); // `def`
        let name_tok = self.bump();
        let name = self.check_target(name_tok)?;
        if late && REX_KEYS.contains(&name.as_str()) {
            return Err(needs(
                R::RezError,
                Some(def_tok.row),
                format!("@late on rex function {name:?}"),
            ));
        }
        // Skip the signature up to the NEWLINE ending the `def` line.
        while self.peek().kind != Kind::Newline {
            if self.peek().kind == Kind::EndMarker {
                return Err(unsupported(self.peek(), "unterminated def"));
            }
            self.bump();
        }
        self.bump();
        // Skip the body: blank lines, then INDENT ... matching DEDENT.
        let save = self.i;
        self.skip_blank();
        if self.peek().kind == Kind::Indent {
            let mut depth = 0usize;
            loop {
                let t = self.bump();
                match t.kind {
                    Kind::Indent => depth += 1,
                    Kind::Dedent => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    Kind::EndMarker => break,
                    _ => {}
                }
            }
        } else {
            // One-line def (`def f(): pass`): the body was on the def line.
            self.i = save;
        }
        let source = function_source(self.toks, self.lines, start_row, decorators.len());
        self.set_global(
            &name,
            Global::Func(Func {
                name: name.clone(),
                decorators,
                late,
                source,
                row: def_tok.row,
            }),
        );
        Ok(())
    }

    /// `with scope("name") [as x]:` followed by an indented block.
    fn with_scope(&mut self) -> Res<()> {
        let with_tok = self.bump();
        let scope_err = |detail: &str| needs(R::Scope, Some(with_tok.row), detail.to_string());
        if !self.is_name(self.peek(), "scope") {
            return Err(scope_err("`with` on something other than scope()"));
        }
        self.bump();
        self.expect_op("(")
            .map_err(|_| scope_err("scope without call"))?;
        let Value::Str(scope_name) = self
            .literal()
            .map_err(|_| scope_err("non-literal scope name"))?
        else {
            return Err(scope_err("non-string scope name"));
        };
        self.expect_op(")")
            .map_err(|_| scope_err("scope() with several arguments"))?;
        let as_name = if self.is_name(self.peek(), "as") {
            self.bump();
            let t = self.bump();
            Some(self.check_target(t)?)
        } else {
            None
        };
        self.expect_op(":")
            .map_err(|_| scope_err("with several items"))?;
        self.expect_newline()
            .map_err(|_| scope_err("single-line with body"))?;
        self.skip_blank();
        if self.peek().kind != Kind::Indent {
            return Err(scope_err("missing with body"));
        }
        self.bump();

        // __enter__: snapshot the globals; then bind the `as` target.
        let snapshot = self.globals.clone();
        let mut scope = self
            .scopes
            .iter()
            .find(|(n, _)| *n == scope_name)
            .map(|(_, s)| s.clone())
            .unwrap_or_default();
        if let Some(n) = &as_name {
            self.set_global(n, Global::ScopeObj);
        }

        loop {
            self.skip_blank();
            let t = self.peek();
            match t.kind {
                Kind::Dedent => {
                    self.bump();
                    break;
                }
                Kind::Str => {
                    self.literal()?;
                    self.expect_newline()?;
                }
                Kind::Name if self.text(t) == "pass" => {
                    self.bump();
                    self.expect_newline()?;
                }
                Kind::Name if self.is_op(self.peek_at(1), "=") => {
                    let t = self.bump();
                    let name = self.check_target(t)?;
                    self.expect_op("=")?;
                    let v = self.literal()?;
                    self.expect_newline()?;
                    self.set_global(&name, Global::Val(v));
                }
                Kind::Name
                    if as_name.as_deref() == Some(self.text(t))
                        && self.is_op(self.peek_at(1), ".") =>
                {
                    self.bump();
                    let mut path = Vec::new();
                    while self.is_op(self.peek(), ".") {
                        self.bump();
                        let a = self.bump();
                        let attr = self.text(a);
                        if a.kind != Kind::Name || !attr.is_ascii() || attr.starts_with("__") {
                            return Err(scope_err("unsupported attribute name"));
                        }
                        path.push(attr.to_string());
                    }
                    self.expect_op("=")
                        .map_err(|_| scope_err("scope attribute read"))?;
                    let v = self.literal()?;
                    self.expect_newline()?;
                    if !set_attr_path(&mut scope, &path, v) {
                        return Err(needs(
                            R::RezError,
                            Some(t.row),
                            "attribute set on a non-attribute value",
                        ));
                    }
                }
                _ => return Err(scope_err("unsupported statement in scope body")),
            }
        }

        // __exit__: capture new or changed (non-dunder, non-_Scope) names.
        let mut updates = Vec::new();
        for (k, g) in &self.globals {
            if k.starts_with("__") && k.ends_with("__") {
                continue;
            }
            let changed = match snapshot.iter().find(|(sk, _)| sk == k) {
                None => true,
                Some((_, old)) => !globals_py_eq(g, old),
            };
            if !changed {
                continue;
            }
            match g {
                Global::ScopeObj => {}
                Global::Val(v) => updates.push((Value::Str(k.clone()), v.clone())),
                Global::Func(_) => return Err(scope_err("function captured by scope")),
            }
        }
        scope.update(updates);
        self.globals = snapshot;
        // _scope_exit: the root scope gets {name: scope.to_dict()}.
        if let Some(slot) = self.scopes.iter_mut().find(|(n, _)| *n == scope_name) {
            slot.1 = scope;
        } else {
            self.scopes.push((scope_name, scope));
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Literals
    // -----------------------------------------------------------------------

    fn literal(&mut self) -> Res<Value> {
        let t = self.peek();
        let text = self.text(t);
        match t.kind {
            Kind::Str => {
                let mut s = String::new();
                while self.peek().kind == Kind::Str {
                    let st = self.bump();
                    s.push_str(&decode_string(self.text(st), st.row)?);
                }
                Ok(Value::Str(s))
            }
            Kind::Number => {
                self.bump();
                parse_number(text, t.row)
            }
            Kind::Name => {
                self.bump();
                match text {
                    "True" => Ok(Value::Bool(true)),
                    "False" => Ok(Value::Bool(false)),
                    "None" => Ok(Value::None),
                    _ => Err(needs(R::NonLiteral, Some(t.row), format!("name {text:?}"))),
                }
            }
            Kind::Op => match text {
                "-" | "+" => {
                    self.bump();
                    let n = self.peek();
                    if n.kind != Kind::Number {
                        return Err(needs(R::NonLiteral, Some(t.row), "unary operator"));
                    }
                    let v = self.literal()?;
                    self.no_trailer()?;
                    match (text, v) {
                        ("+", v) => Ok(v),
                        (_, Value::Int(i)) => i.checked_neg().map(Value::Int).ok_or_else(|| {
                            needs(R::UnsupportedLiteral, Some(t.row), "int overflow")
                        }),
                        (_, Value::Float(f)) => Ok(Value::Float(-f)),
                        _ => unreachable!(),
                    }
                }
                "[" => {
                    self.bump();
                    let (items, _) = self.items("]")?;
                    Ok(Value::List(items))
                }
                "(" => {
                    self.bump();
                    let (items, trailing_comma) = self.items(")")?;
                    if items.len() == 1 && !trailing_comma {
                        Ok(items.into_iter().next().unwrap())
                    } else {
                        Ok(Value::Tuple(items))
                    }
                }
                "{" => {
                    self.bump();
                    self.dict()
                }
                _ => Err(needs(
                    R::NonLiteral,
                    Some(t.row),
                    format!("operator {text:?}"),
                )),
            },
            _ => Err(unsupported(t, "expected a literal")),
        }
        .and_then(|v| {
            self.no_trailer()?;
            Ok(v)
        })
    }

    /// After a literal: refuse calls, subscripts, attribute access and
    /// binary operators (`"a" + "b"`, `[1] * 3`, `"x".format()`, ...).
    fn no_trailer(&self) -> Res<()> {
        let t = self.peek();
        if t.kind == Kind::Op && !matches!(self.text(t), "," | ")" | "]" | "}" | ":" | "=" | ";") {
            return Err(needs(
                R::NonLiteral,
                Some(t.row),
                format!("operator {:?}", self.text(t)),
            ));
        }
        if t.kind == Kind::Op && self.text(t) == ";" {
            return Err(unsupported(t, "semicolon"));
        }
        if matches!(t.kind, Kind::Name | Kind::Number) {
            // e.g. `x if y else z`, `a for a in b`.
            return Err(needs(R::NonLiteral, Some(t.row), "expression"));
        }
        Ok(())
    }

    /// Comma-separated literals up to `close`. Returns (items, trailing comma).
    fn items(&mut self, close: &str) -> Res<(Vec<Value>, bool)> {
        let mut items = Vec::new();
        let mut trailing = false;
        loop {
            self.skip_nl();
            if self.is_op(self.peek(), close) {
                self.bump();
                return Ok((items, trailing));
            }
            if self.is_op(self.peek(), "*") {
                return Err(needs(R::NonLiteral, Some(self.peek().row), "unpacking"));
            }
            items.push(self.literal()?);
            self.skip_nl();
            trailing = false;
            if self.is_op(self.peek(), ",") {
                self.bump();
                trailing = true;
                continue;
            }
            self.skip_nl();
            if self.is_op(self.peek(), close) {
                self.bump();
                return Ok((items, trailing));
            }
            return Err(unsupported(self.peek(), "malformed container literal"));
        }
    }

    fn dict(&mut self) -> Res<Value> {
        let open_row = self.toks.toks[self.i - 1].row;
        let mut d: Vec<(Value, Value)> = Vec::new();
        loop {
            self.skip_nl();
            if self.is_op(self.peek(), "}") {
                self.bump();
                return Ok(Value::Dict(d));
            }
            if self.is_op(self.peek(), "**") {
                return Err(needs(R::NonLiteral, Some(open_row), "dict unpacking"));
            }
            let k = self.literal()?;
            self.skip_nl();
            if !self.is_op(self.peek(), ":") {
                return Err(needs(R::UnsupportedLiteral, Some(open_row), "set literal"));
            }
            self.bump();
            self.skip_nl();
            let v = self.literal()?;
            if !k.is_hashable() {
                return Err(needs(R::RezError, Some(open_row), "unhashable dict key"));
            }
            dict_insert(&mut d, k, v);
            self.skip_nl();
            if self.is_op(self.peek(), ",") {
                self.bump();
                continue;
            }
            self.skip_nl();
            if self.is_op(self.peek(), "}") {
                self.bump();
                return Ok(Value::Dict(d));
            }
            return Err(unsupported(self.peek(), "malformed dict literal"));
        }
    }

    fn skip_nl(&mut self) {
        while matches!(self.peek().kind, Kind::Nl | Kind::Comment) {
            self.i += 1;
        }
    }

    // -----------------------------------------------------------------------
    // Post-processing: what load_py does after exec
    // -----------------------------------------------------------------------

    fn finish(self) -> Res<PackageDefinition> {
        // result = globals minus injected names (refused earlier) ...
        let mut result: Vec<(String, Global)> = self.globals;
        // ... then result.update(scopes.to_dict())
        for (name, scope) in &self.scopes {
            let v = scope.to_dict();
            if let Some(slot) = result.iter_mut().find(|(k, _)| k == name) {
                slot.1 = Global::Val(v);
            } else {
                result.push((name.clone(), Global::Val(v)));
            }
        }
        // process_python_objects: _process then _trim.
        let mut data = Vec::new();
        for (k, g) in result {
            let v = match g {
                Global::ScopeObj => {
                    // Only reachable if the `as` name was never restored,
                    // which can't happen; be safe.
                    return Err(needs(R::Scope, None, "scope object left in globals"));
                }
                Global::Val(v) => v,
                Global::Func(f) => {
                    if f.late || REX_KEYS.contains(&f.name.as_str()) {
                        let source = f.source.map_err(|e| {
                            needs(
                                R::SourceExtraction,
                                Some(f.row),
                                format!("{e:?} for {:?}", f.name),
                            )
                        })?;
                        Value::Source(SourceCode {
                            source,
                            funcname: f.name,
                            decorators: f.decorators,
                            eval_as_function: f.late,
                        })
                    } else if f.name == "preprocess" {
                        return Err(needs(R::Preprocess, Some(f.row), "def preprocess"));
                    } else {
                        continue; // _trim drops plain functions
                    }
                }
            };
            if k.starts_with("__") {
                continue;
            }
            data.push((k, trim(v)?));
        }
        Ok(PackageDefinition { data })
    }
}

/// `_trim` below the top level: recurse into dicts only, drop `__` keys;
/// a non-str key makes rez raise (`k.startswith`).
fn trim(v: Value) -> Res<Value> {
    match v {
        Value::Dict(d) => {
            let mut out = Vec::with_capacity(d.len());
            for (k, v) in d {
                let Value::Str(ks) = &k else {
                    return Err(needs(
                        R::RezError,
                        None,
                        "non-string key in a dict (rez _trim raises)",
                    ));
                };
                if ks.starts_with("__") {
                    continue;
                }
                out.push((k, trim(v)?));
            }
            Ok(Value::Dict(out))
        }
        other => Ok(other),
    }
}

fn globals_py_eq(a: &Global, b: &Global) -> bool {
    match (a, b) {
        (Global::Val(x), Global::Val(y)) => x.py_eq(y),
        (Global::ScopeObj, Global::ScopeObj) => true,
        // Function objects compare by identity. A scope body can't `def`
        // (refused above), so a function present at exit is the very object
        // from the snapshot: equal, hence not captured.
        (Global::Func(_), Global::Func(_)) => true,
        _ => false,
    }
}

/// `scope_obj.a.b.c = v` through `RecursiveAttribute.__getattr__` /
/// `__setattr__`. Returns false where rez would raise AttributeError.
fn set_attr_path(scope: &mut RecAttr, path: &[String], v: Value) -> bool {
    let (last, parents) = path.split_last().unwrap();
    let mut cur = scope;
    for p in parents {
        let key = Value::Str(p.clone());
        let idx = match cur.data.iter().position(|(k, _)| k.py_eq(&key)) {
            Some(i) => i,
            None => {
                cur.data.push((key, ScopeVal::Attr(RecAttr::default())));
                cur.data.len() - 1
            }
        };
        match &mut cur.data[idx].1 {
            ScopeVal::Attr(a) => cur = a,
            ScopeVal::Raw(_) => return false,
        }
    }
    cur.set(Value::Str(last.clone()), ScopeVal::Raw(v));
    true
}

fn unsupported(t: &Tok, detail: impl Into<String>) -> NeedsPython {
    needs(R::UnsupportedStatement, Some(t.row), detail)
}

// ---------------------------------------------------------------------------
// Literal decoding
// ---------------------------------------------------------------------------

fn decode_string(tok: &str, row: u32) -> Res<String> {
    let q = tok.find(['\'', '"']).unwrap();
    let prefix = tok[..q].to_ascii_lowercase();
    if prefix.contains('b') || prefix.contains('f') {
        return Err(needs(
            R::UnsupportedLiteral,
            Some(row),
            "bytes or f-string literal",
        ));
    }
    let raw = prefix.contains('r');
    let body = &tok[q..];
    let triple = body.len() >= 6 && (body.starts_with("'''") || body.starts_with("\"\"\""));
    let inner = if triple {
        &body[3..body.len() - 3]
    } else {
        &body[1..body.len() - 1]
    };
    if raw {
        return Ok(inner.to_string());
    }
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let Some(e) = chars.next() else {
            return Err(needs(R::Tokenize, Some(row), "trailing backslash"));
        };
        match e {
            '\n' => {}
            '\\' => out.push('\\'),
            '\'' => out.push('\''),
            '"' => out.push('"'),
            'a' => out.push('\u{07}'),
            'b' => out.push('\u{08}'),
            'f' => out.push('\u{0c}'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            'v' => out.push('\u{0b}'),
            '0'..='7' => {
                let mut n = e.to_digit(8).unwrap();
                for _ in 0..2 {
                    match chars.peek().and_then(|c| c.to_digit(8)) {
                        Some(d) => {
                            n = n * 8 + d;
                            chars.next();
                        }
                        None => break,
                    }
                }
                out.push(char::from_u32(n).unwrap());
            }
            'x' | 'u' | 'U' => {
                let len = match e {
                    'x' => 2,
                    'u' => 4,
                    _ => 8,
                };
                let hex: String = (0..len).filter_map(|_| chars.next()).collect();
                let n = (hex.len() == len)
                    .then(|| u32::from_str_radix(&hex, 16).ok())
                    .flatten()
                    .filter(|_| hex.bytes().all(|b| b.is_ascii_hexdigit()))
                    .ok_or_else(|| needs(R::Tokenize, Some(row), "bad escape"))?;
                let ch = char::from_u32(n).ok_or_else(|| {
                    needs(
                        R::UnsupportedLiteral,
                        Some(row),
                        "surrogate or out-of-range escape",
                    )
                })?;
                out.push(ch);
            }
            'N' => return Err(needs(R::UnsupportedLiteral, Some(row), "\\N{...} escape")),
            other => {
                // Unknown escapes are kept verbatim (a SyntaxWarning in 3.12+).
                out.push('\\');
                out.push(other);
            }
        }
    }
    Ok(out)
}

fn parse_number(text: &str, row: u32) -> Res<Value> {
    let bad = || needs(R::UnsupportedLiteral, Some(row), format!("number {text:?}"));
    let lower = text.to_ascii_lowercase();
    if lower.ends_with('j') {
        return Err(bad());
    }
    // Underscores: only single, between digits (or right after a base prefix).
    let valid_underscores = {
        let b = lower.as_bytes();
        b.iter().enumerate().all(|(i, &c)| {
            c != b'_'
                || (i > 0
                    && i + 1 < b.len()
                    && b[i + 1].is_ascii_hexdigit()
                    && (b[i - 1].is_ascii_hexdigit()
                        || (i == 2 && matches!(b[1], b'x' | b'o' | b'b'))))
        })
    };
    if !valid_underscores {
        return Err(needs(
            R::Tokenize,
            Some(row),
            "invalid underscore in number",
        ));
    }
    let clean = lower.replace('_', "");
    for (prefix, radix) in [("0x", 16), ("0o", 8), ("0b", 2)] {
        if let Some(digits) = clean.strip_prefix(prefix) {
            return i64::from_str_radix(digits, radix)
                .ok()
                .filter(|_| !digits.is_empty() && !digits.starts_with(['+', '-']))
                .map(Value::Int)
                .ok_or_else(bad);
        }
    }
    if clean.contains(['.', 'e']) {
        if !clean
            .bytes()
            .all(|c| c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'+' | b'-'))
        {
            return Err(bad());
        }
        return clean.parse::<f64>().map(Value::Float).map_err(|_| bad());
    }
    if !clean.bytes().all(|c| c.is_ascii_digit()) {
        return Err(bad());
    }
    if clean.len() > 1 && clean.starts_with('0') && clean.bytes().any(|c| c != b'0') {
        return Err(needs(
            R::Tokenize,
            Some(row),
            "leading zero in decimal literal",
        ));
    }
    clean.parse::<i64>().map(Value::Int).map_err(|_| bad())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(src: &str) -> PackageDefinition {
        match load_package_py(src.as_bytes()) {
            LoadOutcome::Static(d) => d,
            LoadOutcome::NeedsPython(n) => panic!("unexpected NeedsPython: {n}"),
        }
    }

    fn np(src: &str) -> R {
        match load_package_py(src.as_bytes()) {
            LoadOutcome::Static(d) => panic!("unexpected Static: {d:?}"),
            LoadOutcome::NeedsPython(n) => n.reason,
        }
    }

    fn s(x: &str) -> Value {
        Value::Str(x.into())
    }

    #[test]
    fn basic_package() {
        let d = ok("# -*- coding: utf-8 -*-\n\"\"\"doc\"\"\"\nname = 'foo'\nversion = \"1.0\"\nrequires = ['a-1', \"b\"]\nvariants = [['p-2'], ['p-3']]\ntimestamp = 1600129484\n");
        let keys: Vec<&str> = d.data.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            keys,
            vec!["name", "version", "requires", "variants", "timestamp"]
        );
        assert_eq!(
            d.get("requires"),
            Some(&Value::List(vec![s("a-1"), s("b")]))
        );
        assert_eq!(d.get("timestamp"), Some(&Value::Int(1600129484)));
    }

    #[test]
    fn reassignment_keeps_first_position() {
        let d = ok("a = 1\nb = 2\na = 3\n");
        assert_eq!(
            d.data,
            vec![("a".into(), Value::Int(3)), ("b".into(), Value::Int(2))]
        );
    }

    #[test]
    fn functions_become_sourcecode_or_are_dropped() {
        let d = ok("name = 'x'\ndef commands():\n    env.A = 1\ndef helper():\n    pass\n@late()\ndef tools():\n    return ['t']\n");
        let keys: Vec<&str> = d.data.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, vec!["name", "commands", "tools"]);
        let Some(Value::Source(sc)) = d.get("tools") else {
            panic!()
        };
        assert!(sc.eval_as_function);
        assert_eq!(sc.source, "return ['t']");
        assert_eq!(
            sc.decorators,
            vec![Decorator {
                name: "late".into(),
                nargs: None
            }]
        );
    }

    #[test]
    fn scope_block() {
        let d = ok("name = 'x'\nwith scope('config') as c:\n    c.release_packages_path = '/p'\n    c.plugins.a.b = 1\n    other = 2\nversion = '1'\n");
        let keys: Vec<&str> = d.data.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, vec!["name", "version", "config"]);
        let Some(Value::Dict(cfg)) = d.get("config") else {
            panic!()
        };
        let ks: Vec<&Value> = cfg.iter().map(|(k, _)| k).collect();
        assert_eq!(
            ks,
            vec![&s("release_packages_path"), &s("plugins"), &s("other")]
        );
    }

    #[test]
    fn functions_defined_before_scope_are_not_captured() {
        let d = ok("def commands():\n    env.A = 1\nwith scope('config') as c:\n    c.a = 1\n");
        let keys: Vec<&str> = d.data.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, vec!["commands", "config"]);
        assert_eq!(
            d.get("config"),
            Some(&Value::Dict(vec![(s("a"), Value::Int(1))]))
        );
    }

    #[test]
    fn dynamic_constructs_need_python() {
        assert_eq!(np("import os\nname = 'x'\n"), R::Import);
        assert_eq!(
            np("@early()\ndef requires():\n    return []\n"),
            R::EarlyBinding
        );
        assert_eq!(np("if x:\n    name = 'a'\n"), R::ControlFlow);
        assert_eq!(np("name = 'a' + 'b'\n"), R::NonLiteral);
        assert_eq!(np("requires = [x for x in y]\n"), R::NonLiteral);
        assert_eq!(np("name = f'x'\n"), R::UnsupportedLiteral);
        assert_eq!(np("class A:\n    pass\n"), R::ClassDef);
        assert_eq!(np("def preprocess(this, data):\n    pass\n"), R::Preprocess);
        assert_eq!(np("@late()\ndef commands():\n    pass\n"), R::RezError);
        assert_eq!(np("x = 'a'.upper()\n"), R::NonLiteral);
        assert_eq!(np("x = 10**100\n"), R::NonLiteral);
    }

    #[test]
    fn string_escapes() {
        let d = ok("a = 'x\\ty\\\\z\\q'\nb = r'\\n'\nc = 'a' 'b'\nd = '\\x41\\u00e9\\101'\n");
        assert_eq!(d.get("a"), Some(&s("x\ty\\z\\q")));
        assert_eq!(d.get("b"), Some(&s("\\n")));
        assert_eq!(d.get("c"), Some(&s("ab")));
        assert_eq!(d.get("d"), Some(&s("AéA")));
    }

    #[test]
    fn numbers() {
        let d = ok("a = -1\nb = 1_000\nc = 0x1F\nd = 1.5e3\ne = (1,)\nf = (1)\ng = {'k': None, 'k': True}\n");
        assert_eq!(d.get("a"), Some(&Value::Int(-1)));
        assert_eq!(d.get("b"), Some(&Value::Int(1000)));
        assert_eq!(d.get("c"), Some(&Value::Int(31)));
        assert_eq!(d.get("d"), Some(&Value::Float(1500.0)));
        assert_eq!(d.get("e"), Some(&Value::Tuple(vec![Value::Int(1)])));
        assert_eq!(d.get("f"), Some(&Value::Int(1)));
        assert_eq!(
            d.get("g"),
            Some(&Value::Dict(vec![(s("k"), Value::Bool(true))]))
        );
    }

    #[test]
    fn crlf_is_normalised() {
        let d = ok("name = 'x'\r\ndescription = '''a\r\nb'''\r\n");
        assert_eq!(d.get("description"), Some(&s("a\nb")));
    }

    #[test]
    fn dunder_names_trimmed() {
        let d = ok("__version__ = '1'\n_private = 2\nd = {'__x': 1, 'y': 2}\n");
        let keys: Vec<&str> = d.data.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, vec!["_private", "d"]);
        assert_eq!(
            d.get("d"),
            Some(&Value::Dict(vec![(s("y"), Value::Int(2))]))
        );
    }
}
