use crate::description::RerVersion;
use crate::parser::parse_version_range;
use core::fmt;
use lazy_static::lazy_static;
use pubgrub::range::Range;
#[allow(unused_imports)] // Needed to import the Version trait
use pubgrub::version::Version;
use regex::Regex;
use std::fmt::Debug;
use std::hash::{Hash, Hasher};

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
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Requirement {
    pub name: String,
    pub range: Option<Range<RerVersion>>,
    weak_ref: bool,
    conflict: bool,
    sep: char,
    original_name: String,
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
        let orignal_name = input_str.to_string();
        let mut range = None;
        let mut weak_ref = false;
        let conflict = input_str.starts_with('!');
        let mut sep = '-';

        let mut input_str = input_str.to_string();
        if conflict {
            input_str.remove(0);
        } else if input_str.starts_with('~') {
            input_str.remove(0);
            weak_ref = true;
        }

        let name: String = if let Some(m) = SEP_REGEX.find(&input_str) {
            let mut req_str = input_str[m.start()..].to_string();
            if ['-', '@', '#'].contains(&req_str.chars().next().unwrap()) {
                sep = req_str.remove(0);
            }
            if req_str.contains("|") {
                let reqs: Vec<Range<RerVersion>> = req_str
                    .split("|")
                    .into_iter()
                    .map(|x| parse_version_range(x))
                    .collect();
                for req in reqs {
                    if range.is_none() {
                        range = Some(req);
                    } else {
                        range = Some(range.unwrap().union(&req));
                    }
                }
            } else {
                range = Some(parse_version_range(&req_str));
            }
            if conflict {
                range = Some(range.unwrap().negate());
            }
            input_str[..m.start()].to_string()
        } else if conflict {
            input_str
            // '~foo' equates to no effect, so range remains None
        } else {
            range = Some(Range::any());
            input_str
        };

        Requirement {
            name,
            range,
            weak_ref,
            conflict,
            sep,
            original_name,
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
    /// let a = Requirement::from_str("maya-1.2.3+<2.0.0");
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
            return None; // cannot merge across object names
        }

        let merged_range = match (&self.range, &other.range) {
            (None, _) => other.range.clone(),
            (_, None) => self.range.clone(),
            (Some(a), Some(b)) => {
                if self.conflict {
                    if other.conflict {
                        Some(a.union(b)) // Merge conflicts by union
                    } else {
                        Some(Self::difference(b, a)) // Subtract self range from other
                    }
                } else if other.conflict {
                    Some(Self::difference(a, b)) // Subtract other range from self
                } else {
                    Some(a.intersection(b)) // Merge by intersection
                }
            }
        };

        merged_range.map(|range| Requirement {
            name: self.name.clone(),
            range: Some(range),
            weak_ref: self.weak_ref && other.weak_ref, // Assuming similar fields exist
            conflict: self.conflict && other.conflict, // Assuming similar logic applies
            sep: self.sep,                             // Assuming sep remains unchanged
            original_name: self.original_name.clone(), // Assuming a typo fix in field name
        })
    }
    fn difference(a: &Range<RerVersion>, b: &Range<RerVersion>) -> Range<RerVersion> {
        a.intersection(&b.negate())
    }
    /// # is_weak_ref
    ///
    /// ## Description
    ///
    /// Check if the requirement is a weak reference.
    pub fn is_weak_ref(&self) -> bool {
        self.weak_ref
    }
}

#[test]
fn test_requierement() {
    let a = Requirement::from_str("voodoo-1");
    assert_eq!(a.name, "voodoo");
    let v: RerVersion = "1".try_into().unwrap();
    assert_eq!(a.range, Some(Range::between(v.clone(), v.bump())));
    let a = Requirement::from_str("voodoo-1.13.0|2.1.0");
    assert_eq!(a.name, "voodoo");
    let v: RerVersion = "1.13.0".try_into().unwrap();
    let v2: RerVersion = "2.1.0".try_into().unwrap();
    assert_eq!(
        a.range,
        Some(Range::between(v.clone(), v.bump()).union(&Range::between(v2.clone(), v2.bump())))
    );
}

impl fmt::Display for Requirement {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.original_name)
    }
}

impl Hash for Requirement {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.original_name.hash(state);
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
    assert_eq!(c.to_string(), "foo-1");
    let a = Requirement::from_str("foo-1.2");
    let b = Requirement::from_str("foo-1");
    let c = a.merge(&b).unwrap();
    assert_eq!(c.to_string(), "foo-1");
    let a = Requirement::from_str("foo-1.2");
    let b = Requirement::from_str("foo==1.2.2");
    let c = a.merge(&b).unwrap();
    let v_start: RerVersion = "1.2.2".try_into().unwrap();
    assert_eq!(c.get_version_range(), Some(Range::exact(v_start)));
    let a = Requirement::new("foo-1.2");
    let b = Requirement::new("~foo-1");
    let c = a.merge(&b).unwrap();
    let v_start: RerVersion = "1.2".try_into().unwrap();
    let v_end: RerVersion = "1.2_".try_into().unwrap();
    assert_eq!(c.get_version_range(), Some(Range::between(v_start, v_end)));
    let a = Requirement::new("foo-1.2");
    let b = Requirement::new("!foo-1");
    let c = a.merge(&b).unwrap();
    let v_start: RerVersion = "1.2".try_into().unwrap();
    let v_end: RerVersion = "1.2_".try_into().unwrap();
    println!("{:?}", c.get_version_range().unwrap().to_string());
    assert_eq!(c.get_version_range(), Some(Range::between(v_start, v_end)));
}

/// # Requirements
///
/// ## Description
///
/// A list of requirements.
#[derive(Clone, Debug, Default)]
pub struct Requirements(Vec<Requirement>);
impl Requirements {
    pub fn empty() -> Self {
        Requirements(Vec::new())
    }
    pub fn add(&mut self, requirement: Requirement) {
        self.0.push(requirement);
    }
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
    /// # is_empty
    ///
    /// ## Description
    ///
    /// Check if the list of requirements is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn split_weak_ref(&self) -> (Self, Self) {
        let mut weak_ref = Vec::new();
        let mut strong_ref = Vec::new();
        for req in &self.0 {
            if req.is_weak_ref() {
                weak_ref.push(req.clone());
            } else {
                strong_ref.push(req.clone());
            }
        }
        (Requirements(weak_ref), Requirements(strong_ref))
    }
}

impl fmt::Display for Requirements {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut first = true;
        for req in &self.0 {
            if first {
                first = false;
            } else {
                write!(f, ", ")?;
            }
            write!(f, "{}", req)?;
        }
        Ok(())
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

#[test]
fn test_split_weak_ref() {
    let requirements_str = vec!["foo-1.2", "bah-3", "~foo-1"];
    let (weak, strong) = Requirements::from_str(requirements_str).split_weak_ref();
    assert_eq!(weak.0.len(), 1);
    assert_eq!(strong.0.len(), 2);
}
