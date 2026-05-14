//! Faithful port of rez's `Requirement` and `RequirementList`
//! (`rez/src/rez/version/_requirement.py`).

use rer_version::VersionRange;
use rustc_hash::{FxHashMap, FxHashSet};
use std::fmt;
use std::hash::{Hash, Hasher};

/// A requirement for an object, e.g. `foo-5+`, `!foo`, `~foo-1`.
///
/// Mirrors rez's `Requirement`. Two prefixes change the meaning:
/// - `!` — *conflict*: this version range must NOT be present.
/// - `~` — *weak*: "I don't require this, but if present it must be in range".
///   A weak requirement is stored internally as a conflict over the *inverse*
///   of the given range, so the solver only ever distinguishes "conflict" from
///   "normal" — `negate` is purely for faithful display.
#[derive(Debug, Clone)]
pub struct Requirement {
    name: String,
    /// `None` only for the no-effect weak requirement `~foo`. For every other
    /// requirement the range is `Some`.
    range: Option<VersionRange>,
    /// True for a weak (`~`) requirement. Weak requirements are also conflicts.
    negate: bool,
    /// True for a conflict (`!` or `~`) requirement.
    conflict: bool,
    /// Cosmetic name/version separator (`-`, `@` or `#`).
    sep: char,
}

impl Requirement {
    /// Parse a requirement string such as `foo-1.0`, `foo<3`, `!foo`, `~foo-1`.
    ///
    /// # Panics
    ///
    /// Panics on a syntactically invalid version range, matching
    /// [`VersionRange::parse`].
    pub fn parse(s: &str) -> Self {
        let mut conflict = s.starts_with('!');
        let mut negate = false;
        let body: &str = if conflict {
            &s[1..]
        } else if let Some(stripped) = s.strip_prefix('~') {
            negate = true;
            conflict = true;
            stripped
        } else {
            s
        };

        let mut sep = '-';
        let (name, range) = match body.find(|c| matches!(c, '-' | '@' | '#' | '=' | '<' | '>')) {
            Some(i) => {
                let name = body[..i].to_string();
                let mut req_str = &body[i..];
                let first = req_str.chars().next().unwrap();
                if matches!(first, '-' | '@' | '#') {
                    sep = first;
                    req_str = &req_str[1..];
                }
                let parsed = VersionRange::parse(req_str);
                // `~range` is stored as the inverse range; `None` if the
                // parsed range was "any" (inverse of any is empty).
                let range = if negate {
                    parsed.inverse()
                } else {
                    Some(parsed)
                };
                (name, range)
            }
            // Rare case: `~foo` is a requirement with no effect.
            None if negate => (body.to_string(), None),
            None => (body.to_string(), Some(VersionRange::any())),
        };

        Requirement {
            name,
            range,
            negate,
            conflict,
            sep,
        }
    }

    /// Create a requirement directly from a name and range, mirroring rez's
    /// `Requirement.construct`. A `None` range yields an unversioned ("any")
    /// requirement.
    pub fn construct(name: impl Into<String>, range: Option<VersionRange>) -> Self {
        Requirement {
            name: name.into(),
            range: Some(range.unwrap_or_else(VersionRange::any)),
            negate: false,
            conflict: false,
            sep: '-',
        }
    }

    /// Name of the required object.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Version range of the requirement, or `None` for the no-effect `~foo`.
    pub fn range(&self) -> Option<&VersionRange> {
        self.range.as_ref()
    }

    /// True for a conflict requirement (`!foo`, `~foo-1`).
    pub fn is_conflict(&self) -> bool {
        self.conflict
    }

    /// True for a weak requirement (`~foo`).
    pub fn is_weak(&self) -> bool {
        self.negate
    }

    /// Returns true if this requirement conflicts with another — i.e. the two
    /// cannot both be satisfied. Mirrors rez's `Requirement.conflicts_with`.
    pub fn conflicts_with(&self, other: &Requirement) -> bool {
        if self.name != other.name {
            return false;
        }
        let (Some(self_range), Some(other_range)) = (&self.range, &other.range) else {
            return false;
        };
        if self.conflict {
            if other.conflict {
                false
            } else {
                self_range.issuperset(other_range)
            }
        } else if other.conflict {
            other_range.issuperset(self_range)
        } else {
            !self_range.intersects(other_range)
        }
    }

    /// Merge two requirements for the same object, or `None` if they are in
    /// conflict (cannot both be satisfied). Mirrors rez's `Requirement.merged`.
    pub fn merged(&self, other: &Requirement) -> Option<Requirement> {
        if self.name != other.name {
            return None;
        }

        // rez's `_r` helper: copy everything but the range.
        let proto = |src: &Requirement| Requirement {
            name: src.name.clone(),
            range: None,
            negate: src.negate,
            conflict: src.conflict,
            sep: src.sep,
        };

        match (&self.range, &other.range) {
            (None, _) => Some(other.clone()),
            (_, None) => Some(self.clone()),
            (Some(self_range), Some(other_range)) => {
                if self.conflict {
                    if other.conflict {
                        let union = self_range.union(other_range);
                        let mut r = proto(self);
                        r.negate = self.negate && other.negate && !union.is_any();
                        r.range = Some(union);
                        Some(r)
                    } else {
                        let range = other_range.difference(self_range)?;
                        let mut r = proto(other);
                        r.range = Some(range);
                        Some(r)
                    }
                } else if other.conflict {
                    let range = self_range.difference(other_range)?;
                    let mut r = proto(self);
                    r.range = Some(range);
                    Some(r)
                } else {
                    let range = self_range.intersection(other_range)?;
                    let mut r = proto(self);
                    r.range = Some(range);
                    Some(r)
                }
            }
        }
    }
}

impl fmt::Display for Requirement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let pre = if self.negate {
            "~"
        } else if self.conflict {
            "!"
        } else {
            ""
        };
        // For a weak requirement, undo the stored inverse to show the original.
        let range = if self.negate {
            match &self.range {
                Some(r) => r.inverse().unwrap_or_else(VersionRange::any),
                None => VersionRange::any(),
            }
        } else {
            self.range.clone().unwrap_or_else(VersionRange::any)
        };

        write!(f, "{}{}", pre, self.name)?;
        if !range.is_any() {
            let range_str = range.to_string();
            if !matches!(range_str.chars().next(), Some('=') | Some('<') | Some('>')) {
                write!(f, "{}", self.sep)?;
            }
            write!(f, "{}", range_str)?;
        }
        Ok(())
    }
}

// `Eq` and `Hash` are both defined via the string representation so they stay
// consistent (rez hashes on `str` but compares structurally — fine in Python,
// but Rust's hash maps require the two to agree).
impl PartialEq for Requirement {
    fn eq(&self, other: &Self) -> bool {
        self.to_string() == other.to_string()
    }
}

impl Eq for Requirement {}

impl Hash for Requirement {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.to_string().hash(state);
    }
}

/// A list of requirements reduced to optimal form: requirements for the same
/// object are merged, and original order is preserved. Mirrors rez's
/// `RequirementList`.
#[derive(Debug, Clone, Default)]
pub struct RequirementList {
    requirements: Vec<Requirement>,
    by_name: FxHashMap<String, Requirement>,
    /// `Some((existing, incoming))` if two requirements could not be merged.
    conflict: Option<(Requirement, Requirement)>,
    names: Vec<String>,
    conflict_names: Vec<String>,
}

impl RequirementList {
    /// Build a requirement list, merging same-named requirements. If any merge
    /// fails the list is left in a conflicted state — see [`Self::conflict`].
    pub fn new(requirements: Vec<Requirement>) -> Self {
        let mut list = RequirementList::default();

        for req in &requirements {
            match list.by_name.get(req.name()) {
                None => {
                    list.by_name.insert(req.name.clone(), req.clone());
                }
                Some(existing) => match existing.merged(req) {
                    None => {
                        list.conflict = Some((existing.clone(), req.clone()));
                        return list;
                    }
                    Some(merged) => {
                        list.by_name.insert(req.name.clone(), merged);
                    }
                },
            }
        }

        // Build the optimised list in original request order.
        let mut seen = FxHashSet::default();
        for req in &requirements {
            if seen.insert(req.name.clone()) {
                let merged = list.by_name[&req.name].clone();
                if merged.conflict {
                    list.conflict_names.push(req.name.clone());
                } else {
                    list.names.push(req.name.clone());
                }
                list.requirements.push(merged);
            }
        }

        list
    }

    /// The optimised, merged requirement list (in request order).
    pub fn requirements(&self) -> &[Requirement] {
        &self.requirements
    }

    /// The unmergeable requirement pair, if this list is conflicted.
    pub fn conflict(&self) -> Option<&(Requirement, Requirement)> {
        self.conflict.as_ref()
    }

    /// Names of the non-conflict requirements, in request order.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Names of the conflict requirements, in request order.
    pub fn conflict_names(&self) -> &[String] {
        &self.conflict_names
    }

    /// The merged requirement for `name`, if present.
    pub fn get(&self, name: &str) -> Option<&Requirement> {
        self.by_name.get(name)
    }

    /// Iterate the optimised requirement list.
    pub fn iter(&self) -> std::slice::Iter<'_, Requirement> {
        self.requirements.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_plain() {
        let r = Requirement::parse("foo-1.0");
        assert_eq!(r.name(), "foo");
        assert!(!r.is_conflict());
        assert!(!r.is_weak());
        assert_eq!(r.to_string(), "foo-1.0");
    }

    #[test]
    fn test_parse_unversioned() {
        let r = Requirement::parse("foo");
        assert_eq!(r.name(), "foo");
        assert!(r.range().unwrap().is_any());
        assert_eq!(r.to_string(), "foo");
    }

    #[test]
    fn test_parse_conflict() {
        let r = Requirement::parse("!foo");
        assert_eq!(r.name(), "foo");
        assert!(r.is_conflict());
        assert!(!r.is_weak());
        // `!foo` with no range conflicts with all versions — range is "any".
        assert!(r.range().unwrap().is_any());
        assert_eq!(r.to_string(), "!foo");
    }

    #[test]
    fn test_parse_weak_no_effect() {
        // `~foo` is the no-effect requirement: conflict + weak + None range.
        let r = Requirement::parse("~foo");
        assert!(r.is_conflict());
        assert!(r.is_weak());
        assert!(r.range().is_none());
        assert_eq!(r.to_string(), "~foo");
    }

    #[test]
    fn test_parse_weak_versioned() {
        // `~foo-1` is stored as a conflict over the inverse of `1`.
        let r = Requirement::parse("~foo-1");
        assert!(r.is_conflict());
        assert!(r.is_weak());
        let stored = r.range().unwrap();
        // The stored range excludes the `1` superset.
        assert!(!stored.contains(&"1.2".try_into().unwrap()));
        assert!(stored.contains(&"2.0".try_into().unwrap()));
        // ...but it round-trips back to `~foo-1` for display.
        assert_eq!(r.to_string(), "~foo-1");
    }

    #[test]
    fn test_conflicts_with_disjoint_normals() {
        // `foo-4` and `foo-6` cannot both be satisfied.
        let a = Requirement::parse("foo-4");
        let b = Requirement::parse("foo-6");
        assert!(a.conflicts_with(&b));
        assert!(b.conflicts_with(&a));
        // overlapping ranges do not conflict
        let c = Requirement::parse("foo-4+");
        let d = Requirement::parse("foo-4.5");
        assert!(!c.conflicts_with(&d));
        // different names never conflict
        assert!(!a.conflicts_with(&Requirement::parse("bar-6")));
    }

    #[test]
    fn test_merged_examples() {
        // `foo-3+` and `!foo-5+` == `foo-3+<5`
        let m = Requirement::parse("foo-3+")
            .merged(&Requirement::parse("!foo-5+"))
            .unwrap();
        assert_eq!(m.to_string(), "foo-3+<5");
        assert!(!m.is_conflict());

        // `foo-1` and `foo-1.5` == `foo-1.5`
        let m = Requirement::parse("foo-1")
            .merged(&Requirement::parse("foo-1.5"))
            .unwrap();
        assert_eq!(m.to_string(), "foo-1.5");

        // `!foo-2` and `!foo-5` == `!foo-2|5`
        let m = Requirement::parse("!foo-2")
            .merged(&Requirement::parse("!foo-5"))
            .unwrap();
        assert!(m.is_conflict());
        assert_eq!(m.to_string(), "!foo-2|5");
    }

    #[test]
    fn test_merged_conflict_is_none() {
        // `foo-4` and `foo-6` are irreconcilable.
        assert!(Requirement::parse("foo-4")
            .merged(&Requirement::parse("foo-6"))
            .is_none());
        // different names cannot merge
        assert!(Requirement::parse("foo-1")
            .merged(&Requirement::parse("bar-1"))
            .is_none());
    }

    #[test]
    fn test_requirement_list_merges() {
        let list = RequirementList::new(vec![
            Requirement::parse("foo-1"),
            Requirement::parse("bar-2"),
            Requirement::parse("foo-1.5"),
        ]);
        assert!(list.conflict().is_none());
        // foo and bar, in request order, foo merged to 1.5
        assert_eq!(list.requirements().len(), 2);
        assert_eq!(list.get("foo").unwrap().to_string(), "foo-1.5");
        assert_eq!(list.names(), &["foo".to_string(), "bar".to_string()]);
        assert!(list.conflict_names().is_empty());
    }

    #[test]
    fn test_requirement_list_conflict() {
        let list = RequirementList::new(vec![
            Requirement::parse("foo-4"),
            Requirement::parse("foo-6"),
        ]);
        let (a, b) = list.conflict().expect("should be conflicted");
        assert_eq!(a.to_string(), "foo-4");
        assert_eq!(b.to_string(), "foo-6");
    }

    #[test]
    fn test_requirement_list_partitions_conflict_names() {
        let list = RequirementList::new(vec![
            Requirement::parse("foo-1"),
            Requirement::parse("!bar-2"),
        ]);
        assert!(list.conflict().is_none());
        assert_eq!(list.names(), &["foo".to_string()]);
        assert_eq!(list.conflict_names(), &["bar".to_string()]);
    }
}
