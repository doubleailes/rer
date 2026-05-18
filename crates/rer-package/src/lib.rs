//! Hand-rolled lexer for the static subset of a rez `package.py`.
//!
//! Extracts the four solver-relevant fields (`name`, `version`,
//! `requires`, `variants`) by scanning the source line-by-line — no
//! full AST. Successor to the original `rustpython-parser` version,
//! whose ~2 ms/file AST construction dominated the Stage 3 bench.
//! Target for this rewrite: 50–200 μs/file.
//!
//! # Bias toward bailing
//!
//! Any pattern this scanner doesn't recognise produces `None`. The
//! slow path through rez is always available; we only accept files
//! we're confident match rez 1:1 on the four fields.
//!
//! # What's accepted at module scope
//!
//! - `name = "..."` / `version = "..."` (single- or single-quoted
//!   string literal)
//! - `requires = [str, str, ...]` (list/tuple of string literals,
//!   possibly multi-line)
//! - `variants = [[str, ...], ...]` (list/tuple of list/tuple of
//!   string literals)
//! - `def foo(...)` for any `foo` not in solver fields (body skipped)
//! - `with scope("...") as ...: ...` — rez's declarative DSL
//! - Assignments to non-solver fields (RHS skipped)
//! - Module docstring (single `"""..."""` at the top)
//!
//! # What bails
//!
//! - `@early` / `@late` on a solver-field function
//! - Top-level `if` / `for` / `while` / `try` / `class` / `match`
//! - `import` / `from … import`
//! - `with ...` that isn't `with scope(...)`
//! - Non-literal RHS for a solver field
//! - Missing `name` or `version`
//! - Anything else the scanner doesn't explicitly accept

/// The four solver-relevant fields extracted from a `package.py`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageInfo {
    pub name: String,
    pub version: String,
    pub requires: Vec<String>,
    pub variants: Vec<Vec<String>>,
}

const SOLVER_FIELDS: &[&str] = &["name", "version", "requires", "variants"];

/// Try to parse `source` as a statically-resolvable rez `package.py`.
/// Returns `Some(info)` if every top-level statement is recognised
/// and the four solver fields are all literal — `None` otherwise.
pub fn parse_static_package_py(source: &str) -> Option<PackageInfo> {
    let mut p = Parser::new(source);
    p.parse_module()
}

/// Batched, parallel variant of [`parse_static_package_py`]: open and
/// parse every path on a Rayon thread pool, returning a `Vec` aligned
/// with `paths`.
///
/// Each entry in the returned `Vec` is independent — a file that
/// doesn't exist, can't be read, or contains dynamic content all
/// produce `None` at the same index as the input path. No error
/// path: the function never panics on per-file failures.
///
/// Issue #94: the rez integration shim's bottleneck after the static
/// parser landed was the serial Python loop of `open()` calls
/// (~3 s on a typical 132-package Fortiche resolve, 91% of the
/// `_load_family` budget). This call replaces that loop with one
/// `Python::allow_threads`-released batch, so the I/O overlaps
/// across cores.
///
/// Pool size follows Rayon's default (`RAYON_NUM_THREADS` env var or
/// logical core count). Order is preserved regardless of completion
/// order — callers can `zip(paths, results)` after.
pub fn parse_static_packages_py<P>(paths: &[P]) -> Vec<Option<PackageInfo>>
where
    P: AsRef<std::path::Path> + Sync,
{
    use rayon::prelude::*;

    paths
        .par_iter()
        .map(|p| {
            let source = std::fs::read_to_string(p.as_ref()).ok()?;
            parse_static_package_py(&source)
        })
        .collect()
}

// ===========================================================================
// Parser
// ===========================================================================

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(source: &'a str) -> Self {
        Parser {
            src: source.as_bytes(),
            pos: 0,
        }
    }

    fn parse_module(&mut self) -> Option<PackageInfo> {
        let mut name: Option<String> = None;
        let mut version: Option<String> = None;
        let mut requires: Option<Vec<String>> = None;
        let mut variants: Option<Vec<Vec<String>>> = None;
        let mut seen_first_stmt = false;

        loop {
            // Skip blank lines and comment-only lines until we're at the
            // start of a real statement.
            self.skip_blank_and_comment_lines();
            if self.eof() {
                break;
            }
            // We expect to be at column 0 of a real line.
            // If we hit indented content here, something is wrong — bail.
            if self.peek() == Some(b' ') || self.peek() == Some(b'\t') {
                return None;
            }

            // First-statement-only: an unprefixed string literal is a
            // module docstring — eat it and continue.
            if !seen_first_stmt {
                if matches!(self.peek(), Some(b'"' | b'\'')) {
                    // Could be a docstring or a bare-string-expression
                    // (uncommon; we still accept the docstring case at
                    // the top).
                    if self.try_eat_string_statement() {
                        seen_first_stmt = true;
                        continue;
                    }
                    return None;
                }
            }
            seen_first_stmt = true;

            // Decorator: `@deco`, `@deco()`, `@a.b.c(args)`. Skip the
            // entire decorator line and continue — the next loop
            // iteration will see the `def IDENT` that follows. If
            // IDENT is a solver field, the `def` branch bails
            // (whether or not the decorator was `@early` / `@late`).
            // So we don't need to inspect the decorator content itself.
            if self.peek() == Some(b'@') {
                self.skip_statement()?;
                continue;
            }

            // Look for a top-level keyword/identifier.
            let Some(word) = self.peek_ident() else {
                return None;
            };
            match word {
                "import" | "from" | "if" | "for" | "while" | "try" | "match" | "class"
                | "raise" | "return" | "global" | "nonlocal" | "del" | "yield" | "assert"
                | "pass" | "break" | "continue" | "async" => return None,
                "def" => {
                    self.consume_ident("def")?;
                    self.skip_inline_ws();
                    let func_name = self.eat_ident()?;
                    if SOLVER_FIELDS.contains(&func_name.as_str()) {
                        // `def requires(...)` etc. — dynamic; bail.
                        return None;
                    }
                    self.skip_function_body()?;
                }
                "with" => {
                    if !self.try_accept_scope_with()? {
                        return None;
                    }
                }
                _ => {
                    // Assignment: IDENT = RHS
                    let target = self.eat_ident()?;
                    self.skip_inline_ws();
                    // Reject `IDENT: type = value` annotated form for
                    // solver fields (conservative — easier to bail than
                    // to fully parse types).
                    if self.peek() == Some(b':') {
                        if SOLVER_FIELDS.contains(&target.as_str()) {
                            return None;
                        }
                        // Non-solver annotated assignment — skip rest.
                        self.skip_statement()?;
                        continue;
                    }
                    if !self.eat_byte(b'=') {
                        return None;
                    }
                    // Reject `==`, `+=` etc.
                    if matches!(self.peek(), Some(b'=')) {
                        return None;
                    }
                    self.skip_inline_ws();

                    if SOLVER_FIELDS.contains(&target.as_str()) {
                        match target.as_str() {
                            "name" => name = Some(self.eat_string_literal()?),
                            "version" => version = Some(self.eat_string_literal()?),
                            "requires" => requires = Some(self.eat_list_of_strings()?),
                            "variants" => {
                                variants = Some(self.eat_list_of_list_of_strings()?)
                            }
                            _ => unreachable!(),
                        }
                        // After the RHS, the statement should end at
                        // end-of-line (or comment then EOL). Anything
                        // else (e.g. `name = "foo" + "bar"`) is a bail.
                        self.skip_inline_ws();
                        if !self.at_statement_end() {
                            return None;
                        }
                        self.eat_to_eol();
                    } else {
                        // Non-solver field — skip the RHS without
                        // caring what it is.
                        self.skip_statement()?;
                    }
                }
            }
        }

        Some(PackageInfo {
            name: name?,
            version: version?,
            requires: requires.unwrap_or_default(),
            variants: variants.unwrap_or_default(),
        })
    }

    // ---------------------------------------------------------------
    // `with scope("x") as y: ...`
    // ---------------------------------------------------------------

    /// Consume a `with scope(...)` block. Returns `Some(true)` on a
    /// well-formed scope-with that we should accept, `Some(false)` on
    /// a `with` that isn't `with scope(...)`, and `None` on a
    /// pathological body that touches a solver field (poisoned).
    fn try_accept_scope_with(&mut self) -> Option<bool> {
        self.consume_ident("with")?;
        self.skip_inline_ws();
        // Expect `scope`.
        let kw = self.eat_ident()?;
        if kw != "scope" {
            return Some(false);
        }
        self.skip_inline_ws();
        if !self.eat_byte(b'(') {
            return Some(false);
        }
        // Skip the call args by paren counting — we don't actually
        // need the scope name.
        self.skip_balanced(b'(', b')')?;
        self.skip_inline_ws();
        // Optional `as IDENT`.
        if let Some("as") = self.peek_ident() {
            self.consume_ident("as")?;
            self.skip_inline_ws();
            let _as_name = self.eat_ident()?;
            self.skip_inline_ws();
        }
        if !self.eat_byte(b':') {
            return Some(false);
        }
        self.eat_to_eol();
        // Read the body lines. They must be indented (column > 0).
        // The body ends at the next non-blank, non-comment line at
        // column 0. While reading, defensively bail if a line
        // assigns to a solver field at any indentation.
        loop {
            let line_start = self.pos;
            // Skip blank/comment lines (they're part of the body
            // regardless of indent).
            self.skip_blank_and_comment_lines();
            if self.eof() {
                break;
            }
            // Now check if we're indented (in body) or at column 0
            // (end of body).
            if !matches!(self.peek(), Some(b' ' | b'\t')) {
                // Hit column 0 — end of with body.
                self.pos = line_start; // rewind to start of this line
                // The skip_blank_and_comment_lines above might have
                // moved us past blank lines; redo with no-rewind so
                // the outer loop sees the same column-0 statement.
                // Actually we want the outer loop to see the next
                // statement, so just break (it will re-skip blanks).
                break;
            }
            // Indented body line: defensively check for assignment to
            // a solver field (e.g. `name = "x"` somewhere in body).
            let line_bytes = self.peek_logical_line_bytes();
            if line_assigns_to_solver_field(&line_bytes) {
                return None;
            }
            // Skip this logical line.
            self.skip_statement()?;
        }
        Some(true)
    }

    // ---------------------------------------------------------------
    // `def foo(...): body` — skip
    // ---------------------------------------------------------------

    fn skip_function_body(&mut self) -> Option<()> {
        // We've consumed `def IDENT`. Now expect `(...)`.
        self.skip_inline_ws();
        if !self.eat_byte(b'(') {
            return None;
        }
        self.skip_balanced(b'(', b')')?;
        self.skip_inline_ws();
        // Optional `-> annotation`.
        if self.peek() == Some(b'-') {
            // Skip until `:`.
            while let Some(c) = self.peek() {
                if c == b':' {
                    break;
                }
                if c == b'\n' {
                    return None;
                }
                self.pos += 1;
            }
        }
        if !self.eat_byte(b':') {
            return None;
        }
        self.eat_to_eol();
        // Skip indented body lines until next column-0 line.
        loop {
            // Skip blank/comment lines.
            let snap = self.pos;
            self.skip_blank_and_comment_lines();
            if self.eof() {
                break;
            }
            if !matches!(self.peek(), Some(b' ' | b'\t')) {
                self.pos = snap;
                break;
            }
            // Skip the indented line.
            self.skip_statement()?;
        }
        Some(())
    }

    // ---------------------------------------------------------------
    // Solver-field RHS parsers
    // ---------------------------------------------------------------

    /// Parse a single string literal. Accepts `"..."`, `'...'`,
    /// `"""..."""`, `'''...'''`. Returns the decoded body.
    fn eat_string_literal(&mut self) -> Option<String> {
        // Optional string prefix: 'r', 'R', 'u', 'U', 'b', 'B' (no
        // f-strings on solver fields — those are dynamic).
        let prefix_start = self.pos;
        let mut raw = false;
        while let Some(c) = self.peek() {
            match c {
                b'r' | b'R' => {
                    raw = true;
                    self.pos += 1;
                }
                b'u' | b'U' => {
                    self.pos += 1;
                }
                b'b' | b'B' => {
                    // bytes — not a string. Reject for solver fields.
                    return None;
                }
                b'f' | b'F' => {
                    // f-string — dynamic. Reject.
                    return None;
                }
                _ => break,
            }
        }
        let quote = self.peek()?;
        if quote != b'"' && quote != b'\'' {
            // Not a string at all — rewind and fail.
            self.pos = prefix_start;
            return None;
        }
        self.pos += 1;
        // Check for triple-quote.
        let triple = self.peek() == Some(quote) && self.peek_at(1) == Some(quote);
        if triple {
            self.pos += 2;
            return self.eat_triple_string_body(quote, raw);
        }
        self.eat_single_string_body(quote, raw)
    }

    fn eat_single_string_body(&mut self, quote: u8, raw: bool) -> Option<String> {
        let mut out = String::new();
        while let Some(c) = self.peek() {
            match c {
                b'\\' if !raw => {
                    self.pos += 1;
                    let esc = self.peek()?;
                    match esc {
                        b'n' => out.push('\n'),
                        b't' => out.push('\t'),
                        b'r' => out.push('\r'),
                        b'\\' => out.push('\\'),
                        b'\'' => out.push('\''),
                        b'"' => out.push('"'),
                        b'0' => out.push('\0'),
                        b'\n' => {} // line continuation
                        // Unknown escape: pass through both chars
                        // (matches Python's permissive behaviour).
                        other => {
                            out.push('\\');
                            out.push(other as char);
                        }
                    }
                    self.pos += 1;
                }
                b'\\' if raw => {
                    // In raw strings, backslash is literal — but a
                    // closing-quote-after-backslash still ends the
                    // string. Easier to bail for the rare case.
                    out.push('\\');
                    self.pos += 1;
                }
                b'\n' => return None, // unterminated
                c if c == quote => {
                    self.pos += 1;
                    return Some(out);
                }
                _ => {
                    // Take multi-byte UTF-8 sequences as a unit.
                    let start = self.pos;
                    self.advance_one_char();
                    let s = std::str::from_utf8(&self.src[start..self.pos]).ok()?;
                    out.push_str(s);
                }
            }
        }
        None
    }

    fn eat_triple_string_body(&mut self, quote: u8, _raw: bool) -> Option<String> {
        let mut out = String::new();
        while let Some(c) = self.peek() {
            if c == quote
                && self.peek_at(1) == Some(quote)
                && self.peek_at(2) == Some(quote)
            {
                self.pos += 3;
                return Some(out);
            }
            let start = self.pos;
            self.advance_one_char();
            let s = std::str::from_utf8(&self.src[start..self.pos]).ok()?;
            out.push_str(s);
        }
        None
    }

    /// Parse `[s1, s2, ...]` or `(s1, s2, ...)` of string literals.
    /// Handles trailing comma and multi-line layouts.
    fn eat_list_of_strings(&mut self) -> Option<Vec<String>> {
        let opener = self.peek()?;
        let closer = match opener {
            b'[' => b']',
            b'(' => b')',
            _ => return None,
        };
        self.pos += 1;
        let mut out = Vec::new();
        loop {
            self.skip_ws_and_comments();
            if self.peek() == Some(closer) {
                self.pos += 1;
                return Some(out);
            }
            out.push(self.eat_string_literal()?);
            self.skip_ws_and_comments();
            match self.peek()? {
                b',' => {
                    self.pos += 1;
                }
                c if c == closer => {
                    self.pos += 1;
                    return Some(out);
                }
                _ => return None,
            }
        }
    }

    /// Parse `[[s, ...], [s, ...], ...]` — list of list of strings.
    fn eat_list_of_list_of_strings(&mut self) -> Option<Vec<Vec<String>>> {
        let opener = self.peek()?;
        let closer = match opener {
            b'[' => b']',
            b'(' => b')',
            _ => return None,
        };
        self.pos += 1;
        let mut out = Vec::new();
        loop {
            self.skip_ws_and_comments();
            if self.peek() == Some(closer) {
                self.pos += 1;
                return Some(out);
            }
            out.push(self.eat_list_of_strings()?);
            self.skip_ws_and_comments();
            match self.peek()? {
                b',' => {
                    self.pos += 1;
                }
                c if c == closer => {
                    self.pos += 1;
                    return Some(out);
                }
                _ => return None,
            }
        }
    }

    // ---------------------------------------------------------------
    // Statement-skipping (for non-solver assignments)
    // ---------------------------------------------------------------

    /// Skip from the current position to the end of the current
    /// (logical) statement: i.e., until a newline at bracket-depth 0,
    /// outside any string. Handles backslash line continuations.
    fn skip_statement(&mut self) -> Option<()> {
        let mut depth: i32 = 0;
        while let Some(c) = self.peek() {
            match c {
                b'"' | b'\'' => {
                    self.skip_string_at()?;
                }
                b'#' if depth == 0 => {
                    // Comment to EOL.
                    self.eat_to_eol();
                    if depth == 0 {
                        return Some(());
                    }
                }
                b'\\' => {
                    // Line continuation. Handles bare LF and CRLF.
                    self.pos += 1;
                    if self.peek() == Some(b'\r') {
                        self.pos += 1;
                    }
                    if self.peek() == Some(b'\n') {
                        self.pos += 1;
                    }
                }
                b'\n' => {
                    self.pos += 1;
                    if depth == 0 {
                        return Some(());
                    }
                }
                b'(' | b'[' | b'{' => {
                    depth += 1;
                    self.pos += 1;
                }
                b')' | b']' | b'}' => {
                    depth -= 1;
                    if depth < 0 {
                        return None;
                    }
                    self.pos += 1;
                }
                _ => {
                    self.advance_one_char();
                }
            }
        }
        Some(())
    }

    /// Skip past a balanced bracket pair. Assumes the opening bracket
    /// has already been consumed (so we're at depth 1 conceptually).
    fn skip_balanced(&mut self, _open: u8, close: u8) -> Option<()> {
        let mut depth: i32 = 1;
        while let Some(c) = self.peek() {
            match c {
                b'"' | b'\'' => {
                    self.skip_string_at()?;
                }
                b'#' => {
                    self.eat_to_eol();
                }
                b'(' | b'[' | b'{' => {
                    depth += 1;
                    self.pos += 1;
                }
                b')' | b']' | b'}' => {
                    depth -= 1;
                    self.pos += 1;
                    if depth == 0 {
                        if c != close {
                            // Mismatched bracket type — but the
                            // outer caller doesn't actually care,
                            // since we're skipping. Bail to be safe.
                            return None;
                        }
                        return Some(());
                    }
                }
                _ => {
                    self.advance_one_char();
                }
            }
        }
        None
    }

    /// At a `"` or `'`, skip the entire string literal (single or
    /// triple-quoted).
    fn skip_string_at(&mut self) -> Option<()> {
        let quote = self.peek()?;
        debug_assert!(quote == b'"' || quote == b'\'');
        self.pos += 1;
        let triple = self.peek() == Some(quote) && self.peek_at(1) == Some(quote);
        if triple {
            self.pos += 2;
            while let Some(c) = self.peek() {
                if c == quote
                    && self.peek_at(1) == Some(quote)
                    && self.peek_at(2) == Some(quote)
                {
                    self.pos += 3;
                    return Some(());
                }
                if c == b'\\' {
                    self.pos += 1;
                    if self.peek().is_some() {
                        self.advance_one_char();
                    }
                } else {
                    self.advance_one_char();
                }
            }
            None
        } else {
            while let Some(c) = self.peek() {
                match c {
                    b'\\' => {
                        self.pos += 1;
                        if self.peek().is_some() {
                            self.advance_one_char();
                        }
                    }
                    b'\n' => return None,
                    c if c == quote => {
                        self.pos += 1;
                        return Some(());
                    }
                    _ => {
                        self.advance_one_char();
                    }
                }
            }
            None
        }
    }

    /// Try to eat a bare string-statement (typically a docstring at
    /// the top of the module). Consumes the string and the rest of
    /// the line.
    fn try_eat_string_statement(&mut self) -> bool {
        let snap = self.pos;
        if self.eat_string_literal().is_none() {
            self.pos = snap;
            return false;
        }
        self.skip_inline_ws();
        if self.at_statement_end() {
            self.eat_to_eol();
            true
        } else {
            self.pos = snap;
            false
        }
    }

    // ---------------------------------------------------------------
    // Token-level helpers
    // ---------------------------------------------------------------

    fn skip_blank_and_comment_lines(&mut self) {
        loop {
            let snap = self.pos;
            // Skip leading whitespace on this line. `\r` counts —
            // CRLF-terminated files have a `\r` right before the `\n`,
            // and a bare `\r` should never break us.
            while matches!(self.peek(), Some(b' ' | b'\t' | b'\r')) {
                self.pos += 1;
            }
            match self.peek() {
                Some(b'\n') => {
                    self.pos += 1;
                }
                Some(b'#') => {
                    self.eat_to_eol();
                }
                None => return,
                Some(_) => {
                    // Real content — rewind to start of line so the
                    // caller sees the leading whitespace.
                    self.pos = snap;
                    return;
                }
            }
        }
    }

    fn skip_inline_ws(&mut self) {
        // `\r` is transparent — it's just half of a CRLF terminator.
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r')) {
            self.pos += 1;
        }
    }

    fn skip_ws_and_comments(&mut self) {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\n' | b'\r') => self.pos += 1,
                Some(b'#') => self.eat_to_eol(),
                Some(b'\\') => {
                    // Possible line continuation: `\` followed by
                    // newline (LF or CRLF).
                    if self.peek_at(1) == Some(b'\n') {
                        self.pos += 2;
                    } else if self.peek_at(1) == Some(b'\r')
                        && self.peek_at(2) == Some(b'\n')
                    {
                        self.pos += 3;
                    } else {
                        return;
                    }
                }
                _ => return,
            }
        }
    }

    fn eat_to_eol(&mut self) {
        while let Some(c) = self.peek() {
            self.pos += 1;
            if c == b'\n' {
                return;
            }
        }
    }

    fn at_statement_end(&self) -> bool {
        match self.peek() {
            None | Some(b'\n' | b'\r' | b'#') => true,
            _ => false,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.src.get(self.pos + offset).copied()
    }

    fn eat_byte(&mut self, b: u8) -> bool {
        if self.peek() == Some(b) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn eof(&self) -> bool {
        self.pos >= self.src.len()
    }

    /// Look at the next identifier without consuming it. Used to
    /// dispatch on `def` / `with` / etc.
    fn peek_ident(&self) -> Option<&'a str> {
        let bytes = self.src;
        let start = self.pos;
        if !is_ident_start(*bytes.get(start)?) {
            return None;
        }
        let mut end = start + 1;
        while end < bytes.len() && is_ident_continue(bytes[end]) {
            end += 1;
        }
        std::str::from_utf8(&bytes[start..end]).ok()
    }

    /// Consume and return the next identifier.
    fn eat_ident(&mut self) -> Option<String> {
        let ident = self.peek_ident()?;
        let len = ident.len();
        let owned = ident.to_string();
        self.pos += len;
        Some(owned)
    }

    /// Consume a specific keyword. Returns `None` if the next
    /// identifier is anything else.
    fn consume_ident(&mut self, expected: &str) -> Option<()> {
        let id = self.peek_ident()?;
        if id != expected {
            return None;
        }
        self.pos += expected.len();
        Some(())
    }

    /// Advance one UTF-8 code point. Falls back to one byte for
    /// invalid UTF-8 (the source then doesn't parse, but we don't
    /// want to infinite-loop).
    fn advance_one_char(&mut self) {
        let b = self.src[self.pos];
        let len = if b < 0x80 {
            1
        } else if b < 0xC0 {
            1 // invalid lead — skip 1 byte
        } else if b < 0xE0 {
            2
        } else if b < 0xF0 {
            3
        } else {
            4
        };
        self.pos = (self.pos + len).min(self.src.len());
    }

    /// Read the bytes of the current logical line (until newline at
    /// bracket-depth 0). Used to defensively check whether a
    /// with-scope body line assigns to a solver field, without
    /// consuming it from the parser.
    fn peek_logical_line_bytes(&self) -> Vec<u8> {
        let mut p = self.pos;
        let mut depth: i32 = 0;
        let mut out = Vec::new();
        while p < self.src.len() {
            let c = self.src[p];
            match c {
                b'"' | b'\'' => {
                    // Naive: copy through the close quote.
                    out.push(c);
                    p += 1;
                    let quote = c;
                    while p < self.src.len() {
                        let q = self.src[p];
                        out.push(q);
                        p += 1;
                        if q == b'\\' && p < self.src.len() {
                            out.push(self.src[p]);
                            p += 1;
                        } else if q == quote {
                            break;
                        } else if q == b'\n' {
                            break;
                        }
                    }
                }
                b'#' if depth == 0 => {
                    while p < self.src.len() && self.src[p] != b'\n' {
                        p += 1;
                    }
                }
                b'\n' => {
                    if depth == 0 {
                        break;
                    }
                    out.push(c);
                    p += 1;
                }
                b'(' | b'[' | b'{' => {
                    depth += 1;
                    out.push(c);
                    p += 1;
                }
                b')' | b']' | b'}' => {
                    depth -= 1;
                    out.push(c);
                    p += 1;
                }
                _ => {
                    out.push(c);
                    p += 1;
                }
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Free-standing helpers
// ---------------------------------------------------------------------------

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// True if `line` (the bytes of a single logical line) assigns to
/// one of the solver fields at the start (after any leading
/// whitespace). Used inside `with scope(...)` bodies to defensively
/// catch pathological shadowing.
fn line_assigns_to_solver_field(line: &[u8]) -> bool {
    let mut i = 0;
    while i < line.len() && (line[i] == b' ' || line[i] == b'\t') {
        i += 1;
    }
    for field in SOLVER_FIELDS {
        let fb = field.as_bytes();
        if line.len() >= i + fb.len() && &line[i..i + fb.len()] == fb {
            let mut j = i + fb.len();
            // Must be followed by `=` (with optional whitespace), and
            // not `==`, `+=`, etc.
            while j < line.len() && (line[j] == b' ' || line[j] == b'\t') {
                j += 1;
            }
            if j < line.len() && line[j] == b'=' {
                if j + 1 < line.len() && line[j + 1] == b'=' {
                    return false;
                }
                // Make sure what comes after the field name is not a
                // longer identifier (e.g. `nameless = ...`).
                let after = i + fb.len();
                if after < line.len() && is_ident_continue(line[after]) {
                    return false;
                }
                return true;
            }
        }
    }
    false
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    /// Write `content` to a fresh tempfile and return its path. The
    /// `tempfile` crate isn't a dep — keep tests dep-free with a
    /// hand-rolled scratch path under `std::env::temp_dir()`.
    fn write_temp(name: &str, content: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rer-package-test-{}-{}",
            std::process::id(),
            name
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("package.py");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    #[test]
    fn batch_empty_returns_empty() {
        let result: Vec<Option<PackageInfo>> =
            parse_static_packages_py::<&std::path::Path>(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn batch_parses_each_file_independently() {
        let static_path = write_temp(
            "static-1",
            "name = \"app\"\nversion = \"1.0.0\"\nrequires = [\"lib-2\"]\n",
        );
        let dynamic_path = write_temp(
            "dynamic-1",
            "import os\nname = \"foo\"\nversion = \"1.0\"\n",
        );
        let static_path_2 = write_temp(
            "static-2",
            "name = \"lib\"\nversion = \"2.0.0\"\n",
        );

        let paths = vec![&static_path, &dynamic_path, &static_path_2];
        let results = parse_static_packages_py(&paths);

        assert_eq!(results.len(), 3);
        // [0] static → Some
        let r0 = results[0].as_ref().expect("static-1 should parse");
        assert_eq!(r0.name, "app");
        assert_eq!(r0.version, "1.0.0");
        // [1] dynamic (top-level import) → None
        assert!(results[1].is_none(), "dynamic file should bail to None");
        // [2] static → Some
        let r2 = results[2].as_ref().expect("static-2 should parse");
        assert_eq!(r2.name, "lib");
    }

    #[test]
    fn batch_missing_file_becomes_none() {
        // Build a path that doesn't exist; the batched call should map
        // it to None at the matching index, never raise.
        let phantom = std::env::temp_dir().join("rer-package-test-this-does-not-exist/package.py");
        let real = write_temp(
            "real-static",
            "name = \"x\"\nversion = \"1.0\"\n",
        );
        let paths = vec![&phantom, &real];
        let results = parse_static_packages_py(&paths);
        assert_eq!(results.len(), 2);
        assert!(results[0].is_none(), "phantom path must be None");
        assert!(results[1].is_some(), "real path should parse");
    }

    #[test]
    fn batch_preserves_input_order() {
        // Write 16 files alternating valid/dynamic content. The batched
        // call uses par_iter which can complete out of order; the
        // returned Vec must still match the input positions exactly.
        let mut paths: Vec<PathBuf> = Vec::with_capacity(16);
        for i in 0..16 {
            let content = if i % 2 == 0 {
                format!("name = \"pkg{i}\"\nversion = \"1.0\"\n")
            } else {
                // dynamic — top-level if always bails
                format!("name = \"pkg{i}\"\nversion = \"1.0\"\nif True:\n    pass\n")
            };
            paths.push(write_temp(&format!("order-{i}"), &content));
        }
        let results = parse_static_packages_py(&paths);
        assert_eq!(results.len(), 16);
        for (i, r) in results.iter().enumerate() {
            if i % 2 == 0 {
                let r = r.as_ref().expect("even index should be Some");
                assert_eq!(r.name, format!("pkg{i}"));
            } else {
                assert!(r.is_none(), "odd index should be None (dynamic)");
            }
        }
    }

    #[test]
    fn parses_minimal_static() {
        let src = "name = \"foo\"\nversion = \"1.0.0\"\n";
        let info = parse_static_package_py(src).expect("static minimal");
        assert_eq!(info.name, "foo");
        assert_eq!(info.version, "1.0.0");
        assert!(info.requires.is_empty());
        assert!(info.variants.is_empty());
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
        let src = "\"\"\"Some docstring.\"\"\"\n\nname = \"foo\"\nversion = \"1.0\"\n";
        assert!(parse_static_package_py(src).is_some());
    }

    #[test]
    fn ignores_unknown_top_level_fields() {
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

    #[test]
    fn handles_multiline_requires() {
        let src = r#"
name = "foo"
version = "1.0"
requires = [
    "python-3",
    "qt-5",
    # a comment in the middle
    "openexr",
]
"#;
        let info = parse_static_package_py(src).unwrap();
        assert_eq!(info.requires, vec!["python-3", "qt-5", "openexr"]);
    }

    #[test]
    fn handles_trailing_comment_after_assignment() {
        let src = "name = \"foo\"  # this is foo\nversion = \"1.0\"\n";
        let info = parse_static_package_py(src).unwrap();
        assert_eq!(info.name, "foo");
    }

    #[test]
    fn handles_single_quoted_strings() {
        let src = "name = 'foo'\nversion = '1.0'\n";
        let info = parse_static_package_py(src).unwrap();
        assert_eq!(info.name, "foo");
        assert_eq!(info.version, "1.0");
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

if True:
    requires = ["dev-lib"]
else:
    requires = ["prod-lib"]
"#;
        assert!(parse_static_package_py(src).is_none());
    }

    #[test]
    fn bails_on_import() {
        let src = "import sys\n\nname = \"foo\"\nversion = \"1.0\"\n";
        assert!(parse_static_package_py(src).is_none());
    }

    #[test]
    fn bails_on_from_import() {
        let src = "from sys import platform\n\nname = \"foo\"\nversion = \"1.0\"\n";
        assert!(parse_static_package_py(src).is_none());
    }

    #[test]
    fn bails_on_classdef() {
        let src = "name = \"foo\"\nversion = \"1.0\"\n\nclass Helper:\n    pass\n";
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
        let src = "prefix = \"f\"\nname = prefix + \"oo\"\nversion = \"1.0\"\n";
        assert!(parse_static_package_py(src).is_none());
    }

    #[test]
    fn bails_on_function_call_for_requires() {
        let src = "name = \"foo\"\nversion = \"1.0\"\nrequires = build_requires()\n";
        assert!(parse_static_package_py(src).is_none());
    }

    #[test]
    fn bails_on_missing_name() {
        let src = "version = \"1.0\"\n";
        assert!(parse_static_package_py(src).is_none());
    }

    #[test]
    fn bails_on_missing_version() {
        let src = "name = \"foo\"\n";
        assert!(parse_static_package_py(src).is_none());
    }

    #[test]
    fn bails_on_syntax_error() {
        // Unterminated string.
        let src = "name = \"foo\nversion = \"1.0\"\n";
        assert!(parse_static_package_py(src).is_none());
    }

    #[test]
    fn requires_can_be_empty_or_absent() {
        let src = "name = \"foo\"\nversion = \"1.0\"\nrequires = []\n";
        let info = parse_static_package_py(src).unwrap();
        assert!(info.requires.is_empty());
    }

    #[test]
    fn variants_can_be_a_tuple_of_tuples() {
        let src = "name = \"foo\"\nversion = \"1.0\"\nvariants = ((\"a\",), (\"b\",))\n";
        let info = parse_static_package_py(src).unwrap();
        assert_eq!(
            info.variants,
            vec![vec!["a".to_string()], vec!["b".to_string()]]
        );
    }

    #[test]
    fn bails_on_augmented_assignment_to_solver_field() {
        let src = "name = \"foo\"\nname += \"bar\"\nversion = \"1.0\"\n";
        assert!(parse_static_package_py(src).is_none());
    }

    #[test]
    fn handles_crlf_line_endings() {
        // Windows-edited package.py files use `\r\n` line endings.
        // Every Fortiche package.py comes off CIFS-served Samba this
        // way; the lexer must treat `\r` transparently.
        let src = "name = 'foo'\r\nversion = '1.0'\r\nrequires = ['python']\r\n";
        let info = parse_static_package_py(src).expect("CRLF accepted");
        assert_eq!(info.name, "foo");
        assert_eq!(info.version, "1.0");
        assert_eq!(info.requires, vec!["python"]);
    }

    #[test]
    fn handles_crlf_backslash_line_continuation_in_non_solver_field() {
        // Common shape in Windows-edited rez packages: `changelog = \`
        // followed by a CRLF then an indented triple-quoted string.
        // Bumped 50pp on the Fortiche corpus.
        let src = concat!(
            "name = 'foo'\r\n",
            "version = '1.0'\r\n",
            "\r\n",
            "changelog = \\\r\n",
            "    \"\"\"some\r\nmultiline\r\nchangelog\"\"\"\r\n",
            "\r\n",
            "timestamp = 12345\r\n",
        );
        let info = parse_static_package_py(src).expect("CRLF \\<continuation> accepted");
        assert_eq!(info.name, "foo");
    }

    #[test]
    fn ignores_decorator_on_non_solver_function() {
        // `@deprecated def commands(): ...` should be accepted —
        // the decorator is metadata; the function isn't a solver field.
        let src = r#"
name = "foo"
version = "1.0"

@deprecated
def commands():
    env.PATH.append("/bin")
"#;
        let info = parse_static_package_py(src).expect("@deprecated def commands accepted");
        assert_eq!(info.name, "foo");
    }

    #[test]
    fn ignores_decorator_with_args_on_non_solver_function() {
        let src = r#"
name = "foo"
version = "1.0"

@cache(maxsize=1)
def commands():
    pass
"#;
        assert!(parse_static_package_py(src).is_some());
    }

    #[test]
    fn handles_crlf_with_def_body() {
        let src = "name = 'foo'\r\nversion = '1.0'\r\n\r\ndef commands():\r\n    env.PATH.append('{root}/bin')\r\n";
        let info = parse_static_package_py(src).expect("CRLF + def body accepted");
        assert_eq!(info.name, "foo");
    }

    #[test]
    fn bails_on_fstring_for_solver_field() {
        let src = "name = f\"foo\"\nversion = \"1.0\"\n";
        assert!(parse_static_package_py(src).is_none());
    }
}
