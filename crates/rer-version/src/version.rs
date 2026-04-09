use core::cmp::Ordering;
use lazy_static::lazy_static;
use rand::{distributions::Alphanumeric, Rng};
use regex::Regex;
use std::fmt;

lazy_static! {
    static ref ALPHABET_REGEX: Regex =
        Regex::new(r"[a-zA-Z0-9_]+").expect("Can't compile ALPHABET_REGEX regex");
    static ref SEMVER_REGEX: Regex = Regex::new(
        r"^(?P<major>0|[1-9]\d*)\.(?P<minor>0|[1-9]\d*)\.(?P<patch>0|[1-9]\d*)(?:-(?P<prerelease>(?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*)(?:\.(?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*))*))?(?:\+(?P<buildmetadata>[0-9a-zA-Z-]+(?:\.[0-9a-zA-Z-]+)*))?$"
    ).expect("Can't compile SEMVER_REGEX regex");
    static ref NUMERIC_REGEX: Regex = Regex::new(r"[0-9]+").expect("Can't compile NUMERIC_REGEX regex");
}

#[allow(clippy::derive_ord_xor_partial_ord)]
#[derive(Debug, PartialEq, Eq, Clone, Ord)]
struct SubToken {
    s: String,
    n: Option<i64>,
}

impl SubToken {
    fn new(s: &str) -> Self {
        let n = s.parse::<i64>().ok();
        SubToken {
            s: s.to_string(),
            n,
        }
    }

    fn custom_char_order(&self, c: char) -> u8 {
        match c {
            '_' => 0,
            'a'..='z' => 1 + (c as u8 - b'a'),
            'A'..='Z' => 27 + (c as u8 - b'A'),
            '0'..='9' => 53 + (c as u8 - b'0'),
            _ => 255, // Other characters are considered the largest
        }
    }
    fn compare_subtokens(&self, a: &str, b: &str) -> Ordering {
        a.chars()
            .zip(b.chars())
            .map(|(ac, bc)| self.custom_char_order(ac).cmp(&self.custom_char_order(bc)))
            .find(|&ordering| ordering != Ordering::Equal)
            .unwrap_or_else(|| a.len().cmp(&b.len()))
    }
}

#[test]
fn test_subtoken_new() {
    let a = SubToken::new("1");
    assert_eq!(a.s, "1");
    assert_eq!(a.n, Some(1));
    let a = SubToken::new("a");
    assert_eq!(a.s, "a");
    assert_eq!(a.n, None);
    let a = SubToken::new("a1");
    assert_eq!(a.s, "a1");
}

#[allow(clippy::non_canonical_partial_ord_impl)]
impl PartialOrd for SubToken {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match (self.n, other.n) {
            (None, None) => self.compare_subtokens(&self.s, &other.s),
            (None, Some(_)) => Ordering::Less,
            (Some(_), None) => Ordering::Greater,
            (Some(a), Some(b)) => {
                let num_cmp = self.compare_subtokens(&a.to_string(), &b.to_string());
                if num_cmp == Ordering::Equal {
                    self.compare_subtokens(&self.s, &other.s)
                } else {
                    num_cmp
                }
            }
        }
        .into()
    }
}

impl fmt::Display for SubToken {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.s)
    }
}

#[derive(Debug, PartialEq, Eq, Clone, PartialOrd, Ord)]
struct AlphanumericVersionToken {
    subtokens: Vec<SubToken>,
}

impl AlphanumericVersionToken {
    fn new(token: &str) -> Result<Self, &'static str> {
        if !ALPHABET_REGEX.is_match(token) {
            Err("Invalid version token")
        } else {
            Ok(Self {
                subtokens: Self::parse(token),
            })
        }
    }
    // Testing purposes only
    #[allow(dead_code)] // Should be use for test
    fn create_random_token_string() -> Result<Self, &'static str> {
        let s: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(7)
            .map(char::from)
            .collect();
        Self::new(&s)
    }

    fn parse(s: &str) -> Vec<SubToken> {
        let mut subtokens = Vec::new();
        let mut alphas = NUMERIC_REGEX.split(s).peekable();
        let mut numerics = NUMERIC_REGEX.find_iter(s).peekable();

        while alphas.peek().is_some() || numerics.peek().is_some() {
            if let Some(alpha) = alphas.next() {
                if !alpha.is_empty() {
                    subtokens.push(SubToken::new(alpha));
                }
            }
            if let Some(numeric) = numerics.next() {
                subtokens.push(SubToken::new(numeric.as_str()));
            }
        }

        subtokens
    }
}

#[test]
fn test_generate_random_version() {
    AlphanumericVersionToken::create_random_token_string().unwrap();
}

impl fmt::Display for AlphanumericVersionToken {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{}",
            self.subtokens
                .iter()
                .map(ToString::to_string)
                .collect::<String>()
        )
    }
}

impl AlphanumericVersionToken {
    fn lowest() -> Self {
        AlphanumericVersionToken {
            subtokens: vec![SubToken::new("_")],
        }
    }
    fn bump(&self) -> Self {
        let mut next_subtokens = self.subtokens.clone();
        let last = next_subtokens
            .pop()
            .expect("Token should have at least one subtoken");
        if last.n.is_some() {
            next_subtokens.push(last);
            next_subtokens.push(SubToken::new("_"));
        } else {
            let new_last = SubToken::new(&(last.s + "_"));
            next_subtokens.push(new_last);
        }
        AlphanumericVersionToken {
            subtokens: next_subtokens,
        }
    }
}
#[allow(dead_code)] // Need to be checked
impl AlphanumericVersionToken {
    pub fn compare(&self, other: &Self) -> Ordering {
        self.subtokens.iter().cmp(other.subtokens.iter())
    }
}

#[test]
fn test_bump_alpha_num() {
    let a = AlphanumericVersionToken::new("1").unwrap();
    assert_eq!(a.subtokens[0].n, Some(1));
    let b = AlphanumericVersionToken::new("1_").unwrap();
    assert_eq!(b.subtokens[0].n, Some(1));
    assert_eq!(b.subtokens[1].s, "_");
    assert_eq!(b.subtokens[1].n, None);
    assert_eq!(a.bump(), b);
}
/// # RerVersion
///
/// # Description
///
/// A version type that uses a custom versioning scheme. To match the actual Rez versioning scheme,
/// the version string must be alphanumeric and can contain any character except for whitespace.
///
/// ## Examples
/// ```
/// use rer_version::RerVersion;
/// let v: RerVersion = "1.2.3-alpha+beta".try_into().unwrap();
/// assert_eq!(v.to_string(), "1.2.3-alpha+beta");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RerVersion {
    tokens: Vec<AlphanumericVersionToken>,
    seps: Vec<char>,
}
impl RerVersion {
    /// # from_str
    ///
    /// # Description
    ///
    /// Parses a version string into a RerVersion. The version string must be alphanumeric and can
    /// contain any character except for whitespace.
    /// ## Examples
    /// ```
    /// use rer_version::RerVersion;
    /// let v: RerVersion = "1.2.3-alpha+beta".try_into().unwrap();
    /// assert_eq!(v.to_string(), "1.2.3-alpha+beta");
    /// ```
    fn parse_from_string(s: &str) -> Result<Self, &'static str> {
        if !ALPHABET_REGEX.is_match(s) {
            Err("Invalid version token")
        } else {
            let mut tokens = Vec::new();
            let mut seps: Vec<char> = Vec::new();
            let toks = ALPHABET_REGEX.find_iter(s);
            let mut seps_iter = ALPHABET_REGEX.split(s);
            for tok in toks {
                tokens.push(AlphanumericVersionToken::new(tok.as_str())?);
                if let Some(sep) = seps_iter.next() {
                    if let Some(c) = sep.chars().next() {
                        seps.push(c)
                    }
                }
            }
            Ok(Self { tokens, seps })
        }
    }
}
impl RerVersion {
    pub fn lowest() -> Self {
        RerVersion {
            tokens: vec![AlphanumericVersionToken::lowest()],
            seps: vec![],
        }
    }
    pub fn bump(&self) -> Self {
        let mut next_tokens = self.tokens.clone();
        let last = next_tokens
            .pop()
            .expect("Token should have at least one subtoken");
        next_tokens.push(last.bump());
        RerVersion {
            tokens: next_tokens,
            seps: self.seps.clone(),
        }
    }
}
impl fmt::Display for RerVersion {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut s = String::new();
        for (i, token) in self.tokens.iter().enumerate() {
            s.push_str(&token.to_string());
            if i < self.seps.len() {
                s.push(self.seps[i]);
            }
        }
        write!(f, "{}", s)
    }
}

impl TryFrom<&str> for RerVersion {
    type Error = &'static str;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        RerVersion::parse_from_string(s)
    }
}
impl TryFrom<String> for RerVersion {
    type Error = &'static str;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        RerVersion::parse_from_string(&s)
    }
}

#[test]
fn test_from() {
    let v: RerVersion = "1.2.3-alpha+beta".try_into().unwrap();
    assert_eq!(v.to_string(), "1.2.3-alpha+beta");
}
#[test]
fn test_display() {
    let v = RerVersion::parse_from_string("1.2.3-alpha+beta").unwrap();
    assert_eq!(v.to_string(), "1.2.3-alpha+beta");
}
#[test]
fn test_from_str() {
    let v = RerVersion::parse_from_string("1.2.3").unwrap();
    assert_eq!(v.tokens.len(), 3);
    assert_eq!(v.seps.len(), 2);
    let v = RerVersion::parse_from_string("1.2.3-alpha").unwrap();
    assert_eq!(v.tokens.len(), 4);
    assert_eq!(v.seps.len(), 3);
    let v = RerVersion::parse_from_string("1.2.3-alpha+beta").unwrap();
    assert_eq!(v.tokens.len(), 5);
    assert_eq!(v.seps.len(), 4);
    let v: RerVersion = "2.0.0_".try_into().unwrap();
    assert_eq!(v.tokens[2], AlphanumericVersionToken::new("0_").unwrap());
    assert_eq!(v.seps, vec!['.', '.']);
}
#[test]
fn test_order() {
    let a = RerVersion::parse_from_string("1.2.3").unwrap();
    let b = RerVersion::parse_from_string("1.2.4").unwrap();
    assert!(a < b);
    let a = RerVersion::parse_from_string("1.2.3").unwrap();
    let b = RerVersion::parse_from_string("1.2.3-alpha").unwrap();
    assert!(a < b);
    let a = RerVersion::parse_from_string("2.0.0").unwrap();
    let b = RerVersion::parse_from_string("2.0.0_").unwrap();
    assert!(a < b);
}

#[test]
fn test_bump_rez_version() {
    let a: RerVersion = "1.2.3".try_into().unwrap();
    let b: RerVersion = "1.2.3_".try_into().unwrap();
    assert_eq!(a.bump(), b);
}

#[test]
fn test_compare_subtoken() {
    let a = SubToken::new("1");
    let b = SubToken::new("2");
    assert!(a < b);
    let a = SubToken::new("1");
    let b = SubToken::new("1");
    assert!(a == b);
    let a = SubToken::new("1");
    let b = SubToken::new("1a");
    assert!(a >= b);
    let a = SubToken::new("a");
    let b = SubToken::new("1");
    assert!(a < b);
    let a = SubToken::new("a");
    let b = SubToken::new("a");
    assert!(a == b);
    let a = SubToken::new("a");
    let b = SubToken::new("A");
    assert!(a < b);
}
#[test]
fn test_alphanumeric_version_token_compare() {
    let a = AlphanumericVersionToken::new("3").unwrap();
    let b = AlphanumericVersionToken::new("4").unwrap();
    assert!(a < b);
    let a = AlphanumericVersionToken::new("01").unwrap();
    let b = AlphanumericVersionToken::new("1").unwrap();
    assert!(a < b);
    let a = AlphanumericVersionToken::new("beta").unwrap();
    let b = AlphanumericVersionToken::new("1").unwrap();
    assert!(a < b);
    let a = AlphanumericVersionToken::new("a").unwrap();
    let b = AlphanumericVersionToken::new("A").unwrap();
    assert!(a < b);
    let a = AlphanumericVersionToken::new("alpha3").unwrap();
    let b = AlphanumericVersionToken::new("alpha4").unwrap();
    assert!(a < b);
    let a = AlphanumericVersionToken::new("alpha").unwrap();
    let b = AlphanumericVersionToken::new("alpha3").unwrap();
    assert!(a < b);
    let a = AlphanumericVersionToken::new("gamma33").unwrap();
    let b = AlphanumericVersionToken::new("33gamma").unwrap();
    assert!(a < b);
}
