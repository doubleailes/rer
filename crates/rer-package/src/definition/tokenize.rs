//! A Python tokenizer producing the token kinds and positions of CPython's
//! `tokenize` module, as far as rez's loading path observes them:
//!
//! - the module-level parser (`pyload`) needs names, literals and operators;
//! - the `inspect.BlockFinder` port (`source`) needs NEWLINE / NL / COMMENT /
//!   INDENT / DEDENT with their rows and columns.
//!
//! The input must already be decoded and newline-normalised (`\n` only) and
//! end with `\n`. Anything this tokenizer can't classify confidently is an
//! error; the caller turns that into a `NeedsPython` result.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Name,
    Number,
    /// A complete string literal, prefix included. f-strings are one token.
    Str,
    Op,
    Newline,
    Nl,
    Comment,
    Indent,
    Dedent,
    EndMarker,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Tok {
    pub kind: Kind,
    /// Byte range in the source.
    pub start: usize,
    pub end: usize,
    /// 1-based row and 0-based column (in characters) of the token start.
    pub row: u32,
    pub col: u32,
    /// Column of the token end, like `tokenize`'s `end[1]`.
    pub ecol: u32,
}

pub(crate) struct Tokens<'a> {
    pub src: &'a str,
    pub toks: Vec<Tok>,
}

impl<'a> Tokens<'a> {
    pub fn text(&self, t: &Tok) -> &'a str {
        &self.src[t.start..t.end]
    }
}

/// Error with the 1-based row where tokenizing failed.
#[derive(Debug)]
pub(crate) struct TokError {
    pub row: u32,
    pub msg: &'static str,
}

const OPS3: &[&str] = &["**=", "//=", ">>=", "<<=", "...", "!="];
const OPS2: &[&str] = &[
    "->", ":=", "**", "//", ">>", "<<", "<=", ">=", "==", "!=", "+=", "-=", "*=", "/=", "%=", "&=",
    "|=", "^=", "@=",
];
const OPS1: &str = "+-*/%@&|^~<>()[]{},:;.=!";

fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_ascii_alphabetic() || !c.is_ascii()
}

fn is_ident_continue(c: char) -> bool {
    c == '_' || c.is_ascii_alphanumeric() || !c.is_ascii()
}

/// String prefixes recognised by Python 3.13 (case-insensitive).
fn is_string_prefix(p: &str) -> bool {
    matches!(
        p.to_ascii_lowercase().as_str(),
        "" | "r" | "u" | "b" | "br" | "rb" | "f" | "fr" | "rf"
    )
}

struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    /// Byte offset of the start of each line (1-based rows: `line_starts[row-1]`).
    line_starts: Vec<usize>,
    toks: Vec<Tok>,
}

pub(crate) fn tokenize(src: &str) -> Result<Tokens<'_>, TokError> {
    debug_assert!(src.ends_with('\n') || src.is_empty());
    let mut line_starts = vec![0];
    for (i, b) in src.bytes().enumerate() {
        if b == b'\n' && i + 1 < src.len() {
            line_starts.push(i + 1);
        }
    }
    let mut lx = Lexer {
        src,
        bytes: src.as_bytes(),
        line_starts,
        toks: Vec::new(),
    };
    lx.run()?;
    Ok(Tokens { src, toks: lx.toks })
}

impl<'a> Lexer<'a> {
    fn row_col(&self, pos: usize) -> (u32, u32) {
        let row = match self.line_starts.binary_search(&pos) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let col = self.src[self.line_starts[row]..pos].chars().count();
        (row as u32 + 1, col as u32)
    }

    fn push(&mut self, kind: Kind, start: usize, end: usize) {
        let (row, col) = self.row_col(start);
        let (_, ecol) = self.row_col(end);
        self.toks.push(Tok {
            kind,
            start,
            end,
            row,
            col,
            ecol,
        });
    }

    fn err(&self, pos: usize, msg: &'static str) -> TokError {
        TokError {
            row: self.row_col(pos.min(self.src.len().saturating_sub(1))).0,
            msg,
        }
    }

    fn run(&mut self) -> Result<(), TokError> {
        let mut indents: Vec<usize> = vec![0];
        let mut parenlev: i32 = 0;
        let mut continued = false;
        let mut pos = 0usize;
        let n = self.bytes.len();

        while pos < n {
            // ---- start of a physical line ----
            let line_start = pos;
            let line_end = self.src[pos..].find('\n').map(|i| pos + i).unwrap_or(n);

            if parenlev == 0 && !continued {
                // Measure indentation. Tabs / form feeds make the column
                // computation subtle (tabsize 8 + consistency checks): bail.
                let mut col = 0usize;
                while pos < line_end && self.bytes[pos] == b' ' {
                    col += 1;
                    pos += 1;
                }
                if pos < line_end && matches!(self.bytes[pos], b'\t' | b'\x0c') {
                    return Err(self.err(pos, "tab or form feed in indentation"));
                }
                if pos == line_end || self.bytes[pos] == b'#' {
                    // Blank or comment-only line: COMMENT? + NL, no indent change.
                    if pos < line_end {
                        self.push(Kind::Comment, pos, line_end);
                    }
                    self.push(Kind::Nl, line_end, line_end + 1);
                    pos = line_end + 1;
                    continue;
                }
                if col > *indents.last().unwrap() {
                    indents.push(col);
                    self.push(Kind::Indent, line_start, pos);
                }
                while col < *indents.last().unwrap() {
                    indents.pop();
                    if col > *indents.last().unwrap() {
                        return Err(self.err(pos, "unindent does not match any outer level"));
                    }
                    self.push(Kind::Dedent, pos, pos);
                }
            } else {
                continued = false;
            }

            // ---- tokens on this physical line ----
            loop {
                // Recomputed every iteration: a multi-line string token moves
                // `pos` onto a later physical line.
                let line_end = self.src[pos..].find('\n').map(|i| pos + i).unwrap_or(n);
                while pos < line_end && matches!(self.bytes[pos], b' ' | b'\t' | b'\x0c') {
                    pos += 1;
                }
                if pos >= line_end {
                    // End of physical line.
                    let kind = if parenlev > 0 {
                        Kind::Nl
                    } else {
                        Kind::Newline
                    };
                    self.push(kind, line_end, line_end + 1);
                    pos = line_end + 1;
                    break;
                }
                let c = self.src[pos..].chars().next().unwrap();
                if c == '#' {
                    self.push(Kind::Comment, pos, line_end);
                    pos = line_end;
                    continue;
                }
                if c == '\\' {
                    if pos + 1 == line_end {
                        continued = true;
                        pos = line_end + 1;
                        if pos >= n {
                            return Err(self.err(pos, "EOF after line continuation"));
                        }
                        break;
                    }
                    return Err(self.err(pos, "unexpected character after line continuation"));
                }
                if c.is_ascii_digit()
                    || (c == '.' && self.bytes.get(pos + 1).is_some_and(u8::is_ascii_digit))
                {
                    let end = self.scan_number(pos);
                    self.push(Kind::Number, pos, end);
                    pos = end;
                    continue;
                }
                if is_ident_start(c) {
                    let mut end = pos;
                    for ch in self.src[pos..].chars() {
                        if is_ident_continue(ch) {
                            end += ch.len_utf8();
                        } else {
                            break;
                        }
                    }
                    let word = &self.src[pos..end];
                    if end < n && matches!(self.bytes[end], b'\'' | b'"') && is_string_prefix(word)
                    {
                        let send = self.scan_string(pos, end)?;
                        self.push(Kind::Str, pos, send);
                        pos = send;
                    } else {
                        self.push(Kind::Name, pos, end);
                        pos = end;
                    }
                    continue;
                }
                if c == '\'' || c == '"' {
                    let send = self.scan_string(pos, pos)?;
                    self.push(Kind::Str, pos, send);
                    pos = send;
                    continue;
                }
                // Operators.
                let rest = &self.src[pos..];
                let op_len = OPS3
                    .iter()
                    .chain(OPS2)
                    .find(|o| rest.starts_with(**o))
                    .map(|o| o.len())
                    .or_else(|| OPS1.contains(c).then_some(1));
                let Some(len) = op_len else {
                    return Err(self.err(pos, "unrecognised character"));
                };
                match c {
                    '(' | '[' | '{' => parenlev += 1,
                    ')' | ']' | '}' => {
                        parenlev -= 1;
                        if parenlev < 0 {
                            return Err(self.err(pos, "unmatched closing bracket"));
                        }
                    }
                    _ => {}
                }
                self.push(Kind::Op, pos, pos + len);
                pos += len;
            }
            // A multi-line token (string) may have moved `pos` past several
            // lines; the loop continues from wherever `pos` is.
        }

        if parenlev > 0 || continued {
            return Err(self.err(n, "EOF in multi-line statement"));
        }
        for _ in 1..indents.len() {
            self.push(Kind::Dedent, n, n);
        }
        self.push(Kind::EndMarker, n, n);
        Ok(())
    }

    fn scan_number(&self, start: usize) -> usize {
        let b = self.bytes;
        let mut i = start;
        let is_hexlike = b[i] == b'0'
            && b.get(i + 1)
                .is_some_and(|c| matches!(c, b'x' | b'X' | b'o' | b'O' | b'b' | b'B'));
        while i < b.len() {
            let c = b[i];
            if c.is_ascii_alphanumeric() || c == b'_' || c == b'.' {
                // Exponent sign: `1e-5`, `1.5E+3` (not for hex/oct/bin).
                if !is_hexlike
                    && matches!(c, b'e' | b'E')
                    && b.get(i + 1).is_some_and(|n| matches!(n, b'+' | b'-'))
                    && b.get(i + 2).is_some_and(u8::is_ascii_digit)
                {
                    i += 2;
                    continue;
                }
                i += 1;
            } else {
                break;
            }
        }
        i
    }

    /// Scan a string literal whose prefix starts at `start` and whose opening
    /// quote is at `q`. Returns the end offset (exclusive).
    fn scan_string(&self, start: usize, q: usize) -> Result<usize, TokError> {
        let prefix = self.src[start..q].to_ascii_lowercase();
        let raw = prefix.contains('r');
        let fstr = prefix.contains('f');
        let quote = self.bytes[q];
        let triple = self.bytes.get(q + 1) == Some(&quote) && self.bytes.get(q + 2) == Some(&quote);
        let body = if triple { q + 3 } else { q + 1 };
        self.scan_string_body(body, quote, triple, raw, fstr)
            .ok_or_else(|| self.err(start, "unterminated string literal"))
    }

    fn scan_string_body(
        &self,
        mut i: usize,
        quote: u8,
        triple: bool,
        raw: bool,
        fstr: bool,
    ) -> Option<usize> {
        let b = self.bytes;
        loop {
            let c = *b.get(i)?;
            if c == b'\\' {
                // Even in raw strings a backslash escapes the quote / newline
                // for tokenizing purposes.
                if b.get(i + 1).is_none() {
                    return None;
                }
                i += 2;
                let _ = raw;
                continue;
            }
            if c == b'\n' && !triple {
                return None;
            }
            if fstr && c == b'{' {
                if b.get(i + 1) == Some(&b'{') {
                    i += 2;
                    continue;
                }
                i = self.scan_fstring_field(i + 1, triple)?;
                continue;
            }
            if c == quote {
                if !triple {
                    return Some(i + 1);
                }
                if b.get(i + 1) == Some(&quote) && b.get(i + 2) == Some(&quote) {
                    return Some(i + 3);
                }
            }
            i += 1;
        }
    }

    /// Scan an f-string replacement field after its `{`; returns the offset
    /// just past the matching `}`. Nested strings (PEP 701 allows reusing the
    /// outer quote) and brackets are tracked. Comments, and newlines inside
    /// single-quoted f-strings, are refused rather than modelled.
    fn scan_fstring_field(&self, mut i: usize, triple: bool) -> Option<usize> {
        let b = self.bytes;
        let mut depth = 0usize;
        loop {
            let c = *b.get(i)?;
            match c {
                b'\n' if !triple => return None,
                b'#' => return None,
                b'(' | b'[' | b'{' => {
                    depth += 1;
                    i += 1;
                }
                b')' | b']' => {
                    depth = depth.checked_sub(1)?;
                    i += 1;
                }
                b'}' => {
                    if depth == 0 {
                        return Some(i + 1);
                    }
                    depth -= 1;
                    i += 1;
                }
                b'\'' | b'"' => {
                    // Nested string, possibly with its own prefix handled
                    // below (prefix letters are consumed as identifier chars).
                    let mut p = i;
                    while p > 0 && b[p - 1].is_ascii_alphabetic() {
                        p -= 1;
                    }
                    let prefix = self.src[p..i].to_ascii_lowercase();
                    let nested_triple = b.get(i + 1) == Some(&c) && b.get(i + 2) == Some(&c);
                    let body = if nested_triple { i + 3 } else { i + 1 };
                    i = self.scan_string_body(
                        body,
                        c,
                        nested_triple,
                        prefix.contains('r'),
                        prefix.contains('f') && is_string_prefix(&prefix),
                    )?;
                }
                _ => i += 1,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<Kind> {
        tokenize(src).unwrap().toks.iter().map(|t| t.kind).collect()
    }

    #[test]
    fn indent_dedent_and_blank_lines() {
        use Kind::*;
        let k = kinds("def f():\n    x = 1\n\n    # c\ny = 2\n");
        assert_eq!(
            k,
            vec![
                Name, Name, Op, Op, Op, Newline, Indent, Name, Op, Number, Newline, Nl, Comment,
                Nl, Dedent, Name, Op, Number, Newline, EndMarker
            ]
        );
    }

    #[test]
    fn newline_inside_brackets_is_nl() {
        use Kind::*;
        let k = kinds("x = [\n  'a',\n]\n");
        assert_eq!(
            k,
            vec![Name, Op, Op, Nl, Str, Op, Nl, Op, Newline, EndMarker]
        );
    }

    #[test]
    fn triple_quoted_and_fstring() {
        let t = tokenize("x = '''a\nb'''\ny = f\"{d[\"k\"]}\"\n").unwrap();
        let strs: Vec<&str> = t
            .toks
            .iter()
            .filter(|x| x.kind == Kind::Str)
            .map(|x| t.text(x))
            .collect();
        let newline_rows: Vec<u32> = t
            .toks
            .iter()
            .filter(|x| x.kind == Kind::Newline)
            .map(|x| x.row)
            .collect();
        assert_eq!(newline_rows, vec![2, 3]);
        assert_eq!(strs, vec!["'''a\nb'''", "f\"{d[\"k\"]}\""]);
    }

    #[test]
    fn tab_indent_is_refused() {
        assert!(tokenize("def f():\n\tpass\n").is_err());
    }
}
