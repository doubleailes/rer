//! `rez.serialise.load_yaml`: `yaml.load(content, Loader=yaml.FullLoader) or {}`
//! with rez's vendored PyYAML 6.
//!
//! The YAML *syntax* comes from `yaml-rust2`'s event stream; scalar
//! *resolution* (which plain scalars become bool / int / float / null) is
//! done here with PyYAML's YAML 1.1 implicit resolvers and constructors,
//! which differ from YAML 1.2 (`yes`/`on` are booleans, `1_000` is an int,
//! ...). Refused, so rez handles them: anchors / aliases, tags, merge keys,
//! timestamps, non-decimal and sexagesimal ints, multiple documents, tabs,
//! and any non-ASCII byte (rez opens the file in the locale encoding, which
//! rer can't know).

use std::sync::OnceLock;

use regex::Regex;
use yaml_rust2::parser::{Event, Parser};
use yaml_rust2::scanner::TScalarStyle;

use super::value::{
    dict_insert, needs, LoadOutcome, NeedsPython, NeedsPythonReason as R, PackageDefinition, Value,
};

type Res<T> = Result<T, NeedsPython>;

/// Load a `package.yaml` from its raw bytes, like `rez.serialise.load_yaml`.
pub fn load_package_yaml(bytes: &[u8]) -> LoadOutcome {
    match load(bytes) {
        Ok(d) => LoadOutcome::Static(d),
        Err(np) => LoadOutcome::NeedsPython(np),
    }
}

fn yaml_err(line: Option<u32>, detail: impl Into<String>) -> NeedsPython {
    needs(R::Yaml, line, detail)
}

fn load(bytes: &[u8]) -> Res<PackageDefinition> {
    if !bytes.is_ascii() {
        return Err(needs(
            R::Encoding,
            None,
            "non-ASCII package.yaml (locale-dependent decoding)",
        ));
    }
    if let Some(pos) = bytes
        .iter()
        .position(|&b| b == b'\t' || (b < 0x20 && b != b'\n' && b != b'\r') || b == 0x7f)
    {
        let line = bytes[..pos].iter().filter(|&&b| b == b'\n').count() as u32 + 1;
        return Err(yaml_err(Some(line), "tab or control character"));
    }
    // Text mode with universal newlines.
    let text = std::str::from_utf8(bytes)
        .unwrap()
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    if text.lines().any(|l| l.starts_with('%')) {
        return Err(yaml_err(None, "YAML directive"));
    }

    let mut parser = Parser::new_from_str(&text);
    let mut events = Vec::new();
    loop {
        let (ev, mark) = parser
            .next_token()
            .map_err(|e| yaml_err(None, format!("parse error: {e}")))?;
        let line = mark.line() as u32;
        if !text.ends_with('\n')
            && matches!(
                &ev,
                Event::Scalar(_, TScalarStyle::Literal | TScalarStyle::Folded, ..)
            )
        {
            // At end of stream without a final line break PyYAML keeps no
            // trailing newline in a block scalar; yaml-rust2 adds one.
            return Err(yaml_err(
                Some(line),
                "block scalar in a file without final newline",
            ));
        }
        let end = ev == Event::StreamEnd;
        events.push((ev, line));
        if end {
            break;
        }
    }

    let mut it = events.into_iter().peekable();
    let mut root: Option<Value> = None;
    let mut docs = 0;
    while let Some((ev, line)) = it.next() {
        match ev {
            Event::StreamStart | Event::StreamEnd | Event::DocumentEnd => {}
            Event::DocumentStart => {
                docs += 1;
                if docs > 1 {
                    return Err(yaml_err(Some(line), "multiple documents"));
                }
                root = Some(node(&mut it)?);
            }
            other => return Err(yaml_err(Some(line), format!("unexpected event {other:?}"))),
        }
    }

    // `yaml.load(...) or {}`
    let root = root.unwrap_or(Value::None);
    match root {
        Value::Dict(items) if !items.is_empty() => {
            let mut data = Vec::with_capacity(items.len());
            for (k, v) in items {
                let Value::Str(k) = k else {
                    return Err(yaml_err(None, "non-string top-level key"));
                };
                data.push((k, v));
            }
            Ok(PackageDefinition { data })
        }
        v if is_falsy(&v) => Ok(PackageDefinition { data: Vec::new() }),
        _ => Err(needs(
            R::RezError,
            None,
            "top-level YAML value is not a mapping",
        )),
    }
}

fn is_falsy(v: &Value) -> bool {
    match v {
        Value::None => true,
        Value::Bool(b) => !b,
        Value::Int(i) => *i == 0,
        Value::Float(f) => *f == 0.0,
        Value::Str(s) => s.is_empty(),
        Value::List(l) | Value::Tuple(l) => l.is_empty(),
        Value::Dict(d) => d.is_empty(),
        Value::Source(_) => false,
    }
}

fn node<I: Iterator<Item = (Event, u32)>>(it: &mut std::iter::Peekable<I>) -> Res<Value> {
    let Some((ev, line)) = it.next() else {
        return Err(yaml_err(None, "unexpected end of events"));
    };
    match ev {
        Event::Scalar(value, style, anchor, tag) => {
            if anchor != 0 || tag.is_some() {
                return Err(yaml_err(Some(line), "anchor or tag"));
            }
            match style {
                TScalarStyle::Plain => resolve_plain(&value, line),
                TScalarStyle::Literal | TScalarStyle::Folded
                    if value.bytes().all(|b| b == b'\n') =>
                {
                    // yaml-rust2 yields "\n" for an empty clipped block
                    // scalar where PyYAML yields "".
                    Err(yaml_err(Some(line), "empty block scalar"))
                }
                _ => Ok(Value::Str(value)),
            }
        }
        Event::SequenceStart(anchor, tag) => {
            if anchor != 0 || tag.is_some() {
                return Err(yaml_err(Some(line), "anchor or tag"));
            }
            let mut items = Vec::new();
            loop {
                if matches!(it.peek(), Some((Event::SequenceEnd, _))) {
                    it.next();
                    return Ok(Value::List(items));
                }
                items.push(node(it)?);
            }
        }
        Event::MappingStart(anchor, tag) => {
            if anchor != 0 || tag.is_some() {
                return Err(yaml_err(Some(line), "anchor or tag"));
            }
            let mut d = Vec::new();
            loop {
                if matches!(it.peek(), Some((Event::MappingEnd, _))) {
                    it.next();
                    return Ok(Value::Dict(d));
                }
                let k = node(it)?;
                if !k.is_hashable() {
                    return Err(needs(R::RezError, Some(line), "unhashable mapping key"));
                }
                let v = node(it)?;
                dict_insert(&mut d, k, v);
            }
        }
        Event::Alias(_) => Err(yaml_err(Some(line), "alias")),
        other => Err(yaml_err(Some(line), format!("unexpected event {other:?}"))),
    }
}

struct Resolvers {
    bool_: Regex,
    float: Regex,
    int: Regex,
    merge: Regex,
    null: Regex,
    timestamp: Regex,
    value: Regex,
    yaml: Regex,
}

/// PyYAML's `Resolver` implicit resolvers (rez/vendor/yaml/resolver.py),
/// with the verbose-mode whitespace removed.
fn resolvers() -> &'static Resolvers {
    static R: OnceLock<Resolvers> = OnceLock::new();
    R.get_or_init(|| Resolvers {
        bool_: Regex::new(r"^(?:yes|Yes|YES|no|No|NO|true|True|TRUE|false|False|FALSE|on|On|ON|off|Off|OFF)$").unwrap(),
        float: Regex::new(r"^(?:[-+]?(?:[0-9][0-9_]*)\.[0-9_]*(?:[eE][-+][0-9]+)?|\.[0-9][0-9_]*(?:[eE][-+][0-9]+)?|[-+]?[0-9][0-9_]*(?::[0-5]?[0-9])+\.[0-9_]*|[-+]?\.(?:inf|Inf|INF)|\.(?:nan|NaN|NAN))$").unwrap(),
        int: Regex::new(r"^(?:[-+]?0b[0-1_]+|[-+]?0[0-7_]+|[-+]?(?:0|[1-9][0-9_]*)|[-+]?0x[0-9a-fA-F_]+|[-+]?[1-9][0-9_]*(?::[0-5]?[0-9])+)$").unwrap(),
        merge: Regex::new(r"^(?:<<)$").unwrap(),
        null: Regex::new(r"^(?:~|null|Null|NULL|)$").unwrap(),
        timestamp: Regex::new(r"^(?:[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]|[0-9][0-9][0-9][0-9]-[0-9][0-9]?-[0-9][0-9]?(?:[Tt]|[ \t]+)[0-9][0-9]?:[0-9][0-9]:[0-9][0-9](?:\.[0-9]*)?(?:[ \t]*(?:Z|[-+][0-9][0-9]?(?::[0-9][0-9])?))?)$").unwrap(),
        value: Regex::new(r"^(?:=)$").unwrap(),
        yaml: Regex::new(r"^(?:!|&|\*)$").unwrap(),
    })
}

/// Resolve and construct a plain scalar. Resolvers are tried in PyYAML's
/// registration order; each one's first-character gating is implied by its
/// own regex, so testing them in order is equivalent.
fn resolve_plain(s: &str, line: u32) -> Res<Value> {
    let r = resolvers();
    let refuse = |what: &str| yaml_err(Some(line), format!("{what} scalar {s:?}"));
    if r.bool_.is_match(s) {
        return Ok(Value::Bool(matches!(
            s.to_ascii_lowercase().as_str(),
            "yes" | "true" | "on"
        )));
    }
    if r.float.is_match(s) {
        return construct_float(s).ok_or_else(|| refuse("float"));
    }
    if r.int.is_match(s) {
        return construct_int(s).ok_or_else(|| refuse("non-decimal int"));
    }
    if r.merge.is_match(s) {
        return Err(refuse("merge-key"));
    }
    if r.null.is_match(s) {
        return Ok(Value::None);
    }
    if r.timestamp.is_match(s) {
        return Err(refuse("timestamp"));
    }
    if r.value.is_match(s) || r.yaml.is_match(s) {
        return Err(refuse("special"));
    }
    Ok(Value::Str(s.to_string()))
}

/// `SafeConstructor.construct_yaml_int`, decimal branch only.
fn construct_int(s: &str) -> Option<Value> {
    let v = s.replace('_', "");
    let (neg, digits) = match v.as_bytes().first() {
        Some(b'-') => (true, &v[1..]),
        Some(b'+') => (false, &v[1..]),
        _ => (false, &v[..]),
    };
    if digits == "0" {
        return Some(Value::Int(0));
    }
    if digits.starts_with('0') || digits.contains(':') {
        return None; // binary / octal / hex / sexagesimal
    }
    let n: i64 = digits.parse().ok()?;
    Some(Value::Int(if neg { -n } else { n }))
}

/// `SafeConstructor.construct_yaml_float`, without the sexagesimal branch.
fn construct_float(s: &str) -> Option<Value> {
    let v = s.replace('_', "").to_ascii_lowercase();
    let (sign, body) = match v.as_bytes().first() {
        Some(b'-') => (-1.0, &v[1..]),
        Some(b'+') => (1.0, &v[1..]),
        _ => (1.0, &v[..]),
    };
    if body == ".inf" {
        return Some(Value::Float(sign * f64::INFINITY));
    }
    if body == ".nan" {
        return Some(Value::Float(f64::NAN));
    }
    if body.contains(':') {
        return None;
    }
    body.parse::<f64>().ok().map(|f| Value::Float(sign * f))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(src: &str) -> PackageDefinition {
        match load_package_yaml(src.as_bytes()) {
            LoadOutcome::Static(d) => d,
            LoadOutcome::NeedsPython(n) => panic!("unexpected NeedsPython: {n}"),
        }
    }

    fn s(x: &str) -> Value {
        Value::Str(x.into())
    }

    #[test]
    fn rez_test_package_shape() {
        let d = ok("name: foo\nversion: '3.0.1'\n\ndescription: >\n    a foo type thing.\n\nrequires:\n- 'bah-1.2+<2'\n\nvariants:\n- [ 'floob-4.1' ]\n- [ 'floob-2.0' ]\n");
        assert_eq!(d.get("version"), Some(&s("3.0.1")));
        assert_eq!(d.get("description"), Some(&s("a foo type thing.\n")));
        assert_eq!(
            d.get("variants"),
            Some(&Value::List(vec![
                Value::List(vec![s("floob-4.1")]),
                Value::List(vec![s("floob-2.0")])
            ]))
        );
    }

    #[test]
    fn yaml_1_1_resolution() {
        let d = ok("a: yes\nb: off\nc: 1_000\nd: 1.5\ne: ~\nf: 1.2.0\ng: 1e5\nh: -0\ni: .inf\n");
        assert_eq!(d.get("a"), Some(&Value::Bool(true)));
        assert_eq!(d.get("b"), Some(&Value::Bool(false)));
        assert_eq!(d.get("c"), Some(&Value::Int(1000)));
        assert_eq!(d.get("d"), Some(&Value::Float(1.5)));
        assert_eq!(d.get("e"), Some(&Value::None));
        assert_eq!(d.get("f"), Some(&s("1.2.0")));
        assert_eq!(d.get("g"), Some(&s("1e5")));
        assert_eq!(d.get("h"), Some(&Value::Int(0)));
        assert_eq!(d.get("i"), Some(&Value::Float(f64::INFINITY)));
    }

    #[test]
    fn empty_and_falsy_documents_are_empty_dicts() {
        assert!(ok("").data.is_empty());
        assert!(ok("---\n").data.is_empty());
        assert!(ok("0\n").data.is_empty());
        assert!(ok("[]\n").data.is_empty());
    }

    #[test]
    fn refused_constructs() {
        for src in [
            "a: &x 1\nb: *x\n",
            "a: !!str 1\n",
            "a: 2001-12-14\n",
            "a: 0x1F\n",
            "a: 010\n",
            "<<: {a: 1}\n",
            "a: 1\n---\nb: 2\n",
            "a:\tb\n",
            "a: é\n",
            "- a\n",
            "yes: 1\n",
        ] {
            assert!(
                matches!(
                    load_package_yaml(src.as_bytes()),
                    LoadOutcome::NeedsPython(_)
                ),
                "{src:?}"
            );
        }
    }
}
