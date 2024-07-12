use crate::description::RerVersion;
use crate::parser::parse_version_range;
use core::fmt;
use lazy_static::lazy_static;
use pubgrub::range::Range;
#[allow(unused_imports)] // Needed to import the Version trait
use pubgrub::version::Version;
use regex::Regex;

lazy_static! {
    static ref SEP_REGEZ_STR: Regex =
        Regex::new(r"[-@#]").expect("Can't compile SEP_REGEZ_STR regex");
    static ref SEP_REGEX: Regex = Regex::new(r"[-@#=<>]").expect("Can't compile SEP_REGEX regex");
}

/// # Requirement
///
/// ## Description
///
/// A requirement is the representation of the scope of range of a specific package.
/// Mainly it is use to represent a requierement in the list of `requieres` of a
/// package.
#[derive(Debug, Clone)]
pub struct Requirement {
    pub name: String,
    pub range: Option<Range<RerVersion>>,
    negate: bool,
    conflict: bool,
    sep: char,
}

impl Requirement {
    /// # new
    ///
    /// ## Description
    ///
    /// Create a new requirement from a string.
    ///
    /// ## Arguments
    ///
    /// * `s` - The string to parse
    pub fn from_str(input_str: &str) -> Self {
        let mut range = None;
        let mut negate = false;
        let mut conflict = input_str.starts_with('!');
        let mut sep = '-';

        let mut input_str = input_str.to_string();
        if conflict {
            input_str.remove(0);
        } else if input_str.starts_with('~') {
            input_str.remove(0);
            negate = true;
            conflict = true;
        }

        let name: String = if let Some(m) = SEP_REGEX.find(&input_str) {
            let mut req_str = input_str[m.start()..].to_string();
            if ['-', '@', '#'].contains(&req_str.chars().next().unwrap()) {
                sep = req_str.remove(0);
            }

            range = Some(parse_version_range(&req_str));
            if negate {
                range = Some(parse_version_range(&req_str).negate());
            }
            input_str[..m.start()].to_string()
        } else if negate {
            input_str
            // '~foo' equates to no effect, so range remains None
        } else {
            range = Some(Range::any());
            input_str
        };

        Requirement {
            name,
            range,
            negate,
            conflict,
            sep,
        }
    }
    /// # get_pubgrub
    ///
    /// ## Description
    ///
    /// Get the name and the range of the requirement in the pubgrub format.
    /// Use mainly to add a dependency in the pubgrub solver.
    pub fn get_pubgrub(&self) -> (String, Range<RerVersion>) {
        (self.name.clone(), self.range.clone().unwrap())
    }
    /// # get_name
    ///
    /// ## Description
    ///
    /// Get the name of the requirement.
    ///
    /// ## Example
    ///
    /// ```rust
    /// use rer_version::requirement::Requirement;
    /// let a = Requirement::new("maya-1.2.3+<2.0.0");
    /// assert_eq!(a.get_name(), "maya");
    /// ```
    pub fn get_name(&self) -> &str {
        &self.name
    }
    /// # get_version_range
    ///
    /// ## Description
    ///
    /// Get the range of the requirement.
    pub fn get_version_range(&self) -> Option<Range<RerVersion>> {
        self.range.clone()
    }
    /// # merge
    ///
    /// ## Description
    ///
    /// Merge two requirements into one. If the requirements are not compatible, return None.
    pub fn merge(&self, other: &Self) -> Option<Self> {
        if self.name != other.name {
            return None;
        }
        let range = match (&self.range, &other.range) {
            (Some(a), Some(b)) => Some(a.union(b)),
            (Some(a), None) => Some(a.clone()),
            (None, Some(b)) => Some(b.clone()),
            (None, None) => None,
        };
        Some(Requirement {
            name: self.name.clone(),
            range,
            negate: self.negate || other.negate,
            conflict: self.conflict || other.conflict,
            sep: self.sep,
        })
    }
}

#[test]
fn test_requierement() {
    let a = Requirement::from_str("voodoo-1");
    assert_eq!(a.name, "voodoo");
    let v: RerVersion = "1".try_into().unwrap();
    assert_eq!(a.range, Some(Range::between(v.clone(), v.bump())));
}

impl fmt::Display for Requirement {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.conflict {
            write!(f, "!")?;
        }
        if self.negate {
            write!(f, "~")?;
        }
        write!(f, "{}", self.name)?;
        if let Some(range) = &self.range {
            write!(f, "{}{}", self.sep, range)?;
        }
        Ok(())
    }
}
#[test]
fn test_to_string() {
    let a = Requirement::from_str("maya-1");
    assert_eq!(a.to_string(), "maya-1");
    let a = Requirement::from_str("maya");
    assert_eq!(a.to_string(), "maya-∗");
    let a = Requirement::from_str("maya-1.2.3+<2.0.0");
    assert_eq!(a.to_string(), "maya-1.2.3 <= v < 2.0.0");
    let a = Requirement::from_str("maya-1.2.3+");
    assert_eq!(a.to_string(), "maya-1.2.3 <= v");
}

#[test]
fn test_merge_requirement() {
    let a = Requirement::from_str("foo-1.2");
    let b = Requirement::from_str("~foo-1");
    let c = a.merge(&b).unwrap();
    assert_eq!(c.to_string(), "!~foo-[ 1.2, 1.2_ [  [ 1_, ∞ [");
    let a = Requirement::from_str("foo-1.2");
    let b = Requirement::from_str("foo-1");
    let c = a.merge(&b).unwrap();
    assert_eq!(c.to_string(), "foo-1");
    let a = Requirement::from_str("foo-1.2");
    let b = Requirement::from_str("foo==1.2.2");
    let c = a.merge(&b).unwrap();
    assert_eq!(c.to_string(), "foo-1.2");
}

/// # Requirements
///
/// ## Description
///
/// A list of requirements.
#[derive(Clone, Debug)]
pub struct Requirements(Vec<Requirement>);
impl Requirements {
    /// # from_str
    ///
    /// ## Description
    ///
    /// Create a list of requirements from a list of string.
    pub fn from_str(requirements: Vec<&str>) -> Self {
        let requirements = requirements
            .iter()
            .map(|x| Requirement::from_str(x))
            .collect();
        Requirements(requirements)
    }
    /// # merge
    ///
    /// ## Description
    ///
    /// Merge all the requirements into one. If the requirements are not compatible, return None.
    pub fn merge(&self) -> Self {
        if self.0.len() <= 1 {
            return self.clone();
        }
        let mut requirements = self.0.clone();
        let mut i = 0;
        while i < requirements.len() {
            let mut j = i + 1;
            while j < requirements.len() {
                if let Some(req) = requirements[i].merge(&requirements[j]) {
                    requirements[i] = req;
                    requirements.remove(j);
                } else {
                    j += 1;
                }
            }
            i += 1;
        }
        Requirements(requirements)
    }
    /// # to_pubgrub
    ///
    /// ## Description
    ///
    /// Convert the requirements into a list of tuple of name and range.
    pub fn to_pubgrub(&self) -> Vec<(String, Range<RerVersion>)> {
        self.0.iter().map(|x| x.get_pubgrub()).collect()
    }
}

impl Iterator for Requirements {
    type Item = Requirement;
    fn next(&mut self) -> Option<Self::Item> {
        self.0.pop()
    }
}

#[test]
fn test_reduce() {
    let requirements_str = vec!["foo-1.2", "bah-3", "~foo-1"];
    let requirements = Requirements::from_str(requirements_str);
    let requirements = requirements.merge();
    assert_eq!(requirements.0.len(), 2);
    assert_eq!(requirements.0[0].name, "foo");
}
