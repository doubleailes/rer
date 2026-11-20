//! Reproduces the text of `SourceCode.source` for a top-level function,
//! i.e. `rez.utils.sourcecode.SourceCode._init_from_func`:
//!
//! ```python
//! loc = getsourcelines(self.func)[0][len(self.decorators) + 1:]
//! code = dedent(''.join(loc))
//! # ... align lines that start with '#' to the next non-comment line ...
//! code = '\n'.join(codelines).rstrip()
//! code = dedent(code)
//! ```
//!
//! `getsourcelines` is `inspect.getblock(lines[co_firstlineno - 1:])`: an
//! `inspect.BlockFinder` driven by the tokenizer. The port follows the
//! CPython 3.13 `BlockFinder` statement by statement. `getblock` returns the
//! same blocks on CPython 3.8 - 3.13 (rez 3.3 requires >= 3.8); this is
//! checked by `scripts/crossversion_definitions.py`.

use super::tokenize::{Kind, Tokens};
use super::value::py_isspace;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SourceError {
    /// rez's own text processing would raise (IndexError in the comment
    /// alignment loop).
    RezWouldRaise,
}

/// Extract `SourceCode.source` for the function whose first decorator (or
/// `def` line when undecorated) is on 1-based `start_row`.
pub(crate) fn function_source(
    toks: &Tokens<'_>,
    lines: &[&str],
    start_row: u32,
    ndecorators: usize,
) -> Result<String, SourceError> {
    let last = block_last_row(toks, start_row)?;
    let first = (start_row - 1) as usize;
    let block = &lines[first..(first + last as usize).min(lines.len())];
    let body: String = block.iter().skip(ndecorators + 1).copied().collect();

    let code = dedent(&body);
    let mut codelines: Vec<String> = code.split('\n').map(str::to_string).collect();
    let count = codelines.len();
    for i in 0..count {
        if !codelines[i].starts_with('#') {
            continue;
        }
        // `nextindex = i + 1 if i < linescount else i - 1` — always i + 1.
        let mut nextindex = i + 1;
        let mut nextline = codelines
            .get(nextindex)
            .ok_or(SourceError::RezWouldRaise)?
            .clone();
        while nextline.starts_with('#') {
            nextline = codelines
                .get(nextindex)
                .ok_or(SourceError::RezWouldRaise)?
                .clone();
            nextindex = if nextindex < count {
                nextindex + 1
            } else {
                nextindex - 1
            };
        }
        let stripped = nextline.trim_start_matches(py_isspace);
        let firstchar = nextline.chars().count() - stripped.chars().count();
        let prefix: String = nextline.chars().take(firstchar).collect();
        codelines[i] = format!("{prefix}{}", codelines[i]);
    }
    let joined = codelines.join("\n");
    Ok(dedent(joined.trim_end_matches(py_isspace)))
}

/// `inspect.BlockFinder` (CPython 3.13) over the tokens from `start_row`
/// onwards. Returns `blockfinder.last`, relative to `start_row` (1-based).
fn block_last_row(toks: &Tokens<'_>, start_row: u32) -> Result<u32, SourceError> {
    let mut indent: i32 = 0;
    let mut singleline = false;
    let mut started = false;
    let mut passline = false;
    let mut indecorator = false;
    let mut last: u32 = 1;
    let mut body_col0: Option<u32> = None;

    // `getblock(lines[lnum:])` tokenizes from a fresh state, so the DEDENTs
    // that close the *previous* block (placed on our first row) don't exist.
    let first = toks
        .toks
        .iter()
        .position(|t| t.row >= start_row && t.kind != Kind::Dedent)
        .unwrap_or(toks.toks.len());
    for t in &toks.toks[first..] {
        let rel = t.row - start_row + 1;
        let text = toks.text(t);
        if !started && !indecorator {
            if matches!(t.kind, Kind::Indent | Kind::Comment | Kind::Nl) {
            } else if text == "async" {
            } else if t.kind == Kind::Op && text == "@" {
                indecorator = true;
            } else {
                singleline = !(t.kind == Kind::Name && (text == "def" || text == "class"));
                started = true;
            }
            passline = true;
        } else if t.kind == Kind::Newline {
            passline = false;
            last = rel;
            if singleline {
                break;
            }
            if indecorator {
                indecorator = false;
            }
        } else if passline {
        } else if t.kind == Kind::Indent {
            if body_col0.is_none() && started {
                body_col0 = Some(t.ecol);
            }
            indent += 1;
            passline = true;
        } else if t.kind == Kind::Dedent {
            indent -= 1;
            if indent <= 0 {
                break;
            }
        } else if t.kind == Kind::Comment {
            if body_col0.is_some_and(|c0| t.col >= c0) {
                last = rel;
            }
        } else if indent == 0 && !matches!(t.kind, Kind::Comment | Kind::Nl) {
            break;
        }
    }
    Ok(last)
}

/// `textwrap.dedent`.
pub(crate) fn dedent(text: &str) -> String {
    // `_whitespace_only_re.sub('', text)`: lines of only spaces/tabs -> "".
    let lines: Vec<&str> = text
        .split('\n')
        .map(|l| {
            if !l.is_empty() && l.bytes().all(|b| b == b' ' || b == b'\t') {
                ""
            } else {
                l
            }
        })
        .collect();

    // `_leading_whitespace_re.findall`: indents of lines with a char that
    // isn't space/tab/newline.
    let mut margin: Option<&str> = None;
    for l in &lines {
        let ws_len = l.len() - l.trim_start_matches([' ', '\t']).len();
        if ws_len == l.len() {
            continue; // empty line: no match
        }
        let indent = &l[..ws_len];
        margin = Some(match margin {
            None => indent,
            Some(m) if indent.starts_with(m) => m,
            Some(m) if m.starts_with(indent) => indent,
            Some(m) => {
                let common = m
                    .bytes()
                    .zip(indent.bytes())
                    .take_while(|(a, b)| a == b)
                    .count();
                &m[..common]
            }
        });
    }
    match margin {
        Some(m) if !m.is_empty() => lines
            .iter()
            .map(|l| l.strip_prefix(m).unwrap_or(l))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => lines.join("\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::super::tokenize::tokenize;
    use super::*;

    fn src_of(text: &str, start_row: u32, ndec: usize) -> Result<String, SourceError> {
        let toks = tokenize(text).unwrap();
        let lines: Vec<&str> = text.split_inclusive('\n').collect();
        function_source(&toks, &lines, start_row, ndec)
    }

    #[test]
    fn simple_commands_body() {
        let s = "name = 'x'\ndef commands():\n    env.A = 1\n    if x:\n        env.B = 2\n\nversion = '1'\n";
        assert_eq!(src_of(s, 2, 0).unwrap(), "env.A = 1\nif x:\n    env.B = 2");
    }

    #[test]
    fn decorated_late() {
        let s = "@late()\ndef requires():\n    return ['a']\n";
        assert_eq!(src_of(s, 1, 1).unwrap(), "return ['a']");
    }

    #[test]
    fn comment_realignment() {
        let s = "def commands():\n    if x:\n# c\n        env.A = 1\n";
        assert_eq!(src_of(s, 1, 0).unwrap(), "if x:\n    # c\n    env.A = 1");
    }

    #[test]
    fn one_liner_is_empty_source() {
        assert_eq!(src_of("def commands(): pass\nx = 1\n", 1, 0).unwrap(), "");
    }

    #[test]
    fn trailing_comment_at_body_indent_is_included() {
        let s = "def commands():\n    env.A = 1\n    # env.B = 2\n# col0\nx = 1\n";
        assert_eq!(src_of(s, 1, 0).unwrap(), "env.A = 1\n# env.B = 2");
    }

    #[test]
    fn dedent_matches_textwrap() {
        assert_eq!(dedent("  a\n    b\n \n  c"), "a\n  b\n\nc");
        assert_eq!(dedent("\ta\n  b"), "\ta\n  b");
    }
}
