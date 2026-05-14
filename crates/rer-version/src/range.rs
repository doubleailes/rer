//! Faithful port of rez's `VersionRange` semantics.
//!
//! rez's solver relies on a specific contract that `version_ranges::Ranges`
//! does not express directly — chiefly that `intersection`, the `-` operator
//! and `inverse` return `None` to signal "empty / no result", and that there
//! is a distinguished "any" range. This module wraps `Ranges<RerVersion>` and
//! re-exposes rez's API (`rez/src/rez/version/_version.py`, class
//! `VersionRange`) so the rest of the solver port can mirror the Python code
//! closely.

use crate::requirement::parser::parse_version_range;
use crate::version::RerVersion;
use core::fmt;
use std::ops::Bound;
use version_ranges::Ranges;

/// A set of one or more contiguous version ranges, with rez's semantics.
///
/// Construct from a rez range string with [`VersionRange::parse`], from a
/// single version with [`VersionRange::from_version`], or from an existing
/// [`Ranges`] with [`VersionRange::from_ranges`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionRange(Ranges<RerVersion>);

impl VersionRange {
    /// The "any" range — contains every version. Mirrors rez's empty-string range.
    pub fn any() -> Self {
        VersionRange(Ranges::full())
    }

    /// The empty range — contains no version. rez represents "empty" as `None`
    /// at the API boundary, but an empty `VersionRange` is still useful
    /// internally (e.g. as a `from_versions` accumulator).
    pub fn empty() -> Self {
        VersionRange(Ranges::empty())
    }

    /// Parse a rez range string such as `"3"`, `"1+<2"`, `"==1.0"`, `"2|6+"`.
    ///
    /// `|`-separated alternatives are unioned together. The empty string is the
    /// "any" range.
    ///
    /// # Panics
    ///
    /// Panics on a syntactically invalid range string, matching the existing
    /// `parse_version_range` behaviour.
    pub fn parse(s: &str) -> Self {
        let mut range: Option<Ranges<RerVersion>> = None;
        for part in s.split('|') {
            let part_range = parse_version_range(part);
            range = Some(match range {
                None => part_range,
                Some(acc) => acc.union(&part_range),
            });
        }
        VersionRange(range.unwrap_or_else(Ranges::full))
    }

    /// Wrap a raw [`Ranges`] (already produced by the requirement parser).
    pub fn from_ranges(ranges: Ranges<RerVersion>) -> Self {
        VersionRange(ranges)
    }

    /// Borrow the underlying [`Ranges`].
    pub fn as_ranges(&self) -> &Ranges<RerVersion> {
        &self.0
    }

    /// Consume into the underlying [`Ranges`].
    pub fn into_ranges(self) -> Ranges<RerVersion> {
        self.0
    }

    /// rez `VersionRange.from_version` with `op=None`: the "superset" range
    /// `[version, version.next())`, e.g. `3` contains `3`, `3.0`, `3.1.4`.
    pub fn from_version(version: &RerVersion) -> Self {
        VersionRange(Ranges::between(version.clone(), version.bump()))
    }

    /// rez `VersionRange.from_versions`: a range containing exactly the given
    /// versions and nothing else (e.g. `==3|==4|==5.1`).
    ///
    /// Built in a single pass via `Ranges`'s `FromIterator` rather than folding
    /// `union` over singletons (which reallocated and was O(n²)) — this is hot,
    /// it backs `_PackageScope._update`.
    pub fn from_versions<I: IntoIterator<Item = RerVersion>>(versions: I) -> Self {
        VersionRange(
            versions
                .into_iter()
                .map(|v| (Bound::Included(v.clone()), Bound::Included(v)))
                .collect(),
        )
    }

    /// rez `VersionRange.is_any`: true for the range that contains all versions.
    pub fn is_any(&self) -> bool {
        self.0 == Ranges::full()
    }

    /// True if this range contains no version.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// rez `VersionRange.intersection` (`&`): `None` if the ranges are disjoint.
    pub fn intersection(&self, other: &Self) -> Option<Self> {
        let result = self.0.intersection(&other.0);
        if result.is_empty() {
            None
        } else {
            Some(VersionRange(result))
        }
    }

    /// rez `VersionRange.union` (`|`): always succeeds.
    pub fn union(&self, other: &Self) -> Self {
        VersionRange(self.0.union(&other.0))
    }

    /// rez `VersionRange.inverse` (`~`): `None` if and only if this is the
    /// "any" range (whose inverse would be empty).
    pub fn inverse(&self) -> Option<Self> {
        if self.is_any() {
            None
        } else {
            Some(VersionRange(self.0.complement()))
        }
    }

    /// rez `VersionRange.__sub__` (`-`): `self & ~other`. `None` if `other` is
    /// the "any" range, or if the result is empty.
    pub fn difference(&self, other: &Self) -> Option<Self> {
        match other.inverse() {
            None => None,
            Some(inverse) => self.intersection(&inverse),
        }
    }

    /// rez `VersionRange.issuperset`: true if `other` is fully contained here.
    pub fn issuperset(&self, other: &Self) -> bool {
        other.0.subset_of(&self.0)
    }

    /// rez `VersionRange.issubset`: true if `self` is fully contained in `other`.
    pub fn issubset(&self, other: &Self) -> bool {
        other.issuperset(self)
    }

    /// rez `VersionRange.intersects`: true if the ranges share any version.
    pub fn intersects(&self, other: &Self) -> bool {
        !self.0.intersection(&other.0).is_empty()
    }

    /// rez `VersionRange.contains_version` / `version in range`.
    pub fn contains(&self, version: &RerVersion) -> bool {
        self.0.contains(version)
    }

    /// rez `VersionRange.to_versions`: the exact (single-version) bounds present
    /// in this range, or `None` if there are none. Non-exact bounds are ignored,
    /// matching rez.
    pub fn to_versions(&self) -> Option<Vec<RerVersion>> {
        let mut versions = Vec::new();
        for (lower, upper) in self.0.iter() {
            if let (Bound::Included(low), Bound::Included(high)) = (lower, upper) {
                if low == high {
                    versions.push(low.clone());
                }
            }
        }
        if versions.is_empty() {
            None
        } else {
            Some(versions)
        }
    }

    /// rez `VersionRange.span`: the smallest contiguous range that is a superset
    /// of this range (e.g. the span of `2+<4|6+<8` is `2+<8`).
    pub fn span(&self) -> Self {
        match self.0.bounding_range() {
            None => VersionRange::empty(),
            Some((lower, upper)) => VersionRange(Ranges::from_range_bounds((
                clone_bound(lower),
                clone_bound(upper),
            ))),
        }
    }

    /// rez `VersionRange.split`: one [`VersionRange`] per contiguous sub-range
    /// (e.g. `3|5+` splits into `["3", "5+"]`).
    pub fn split(&self) -> Vec<Self> {
        self.0
            .iter()
            .map(|(lower, upper)| {
                VersionRange(Ranges::from_range_bounds((lower.clone(), upper.clone())))
            })
            .collect()
    }
}

/// Clone a `Bound<&V>` into an owned `Bound<V>`.
fn clone_bound(bound: Bound<&RerVersion>) -> Bound<RerVersion> {
    match bound {
        Bound::Included(v) => Bound::Included(v.clone()),
        Bound::Excluded(v) => Bound::Excluded(v.clone()),
        Bound::Unbounded => Bound::Unbounded,
    }
}

/// Format one contiguous segment in rez's range syntax, mirroring rez's
/// `_Bound.__str__` (`rez/src/rez/version/_version.py:506`).
fn format_segment(lower: &Bound<RerVersion>, upper: &Bound<RerVersion>) -> String {
    // 1. Infinite upper bound -> just the lower bound string.
    if matches!(upper, Bound::Unbounded) {
        return match lower {
            Bound::Unbounded => String::new(),
            Bound::Included(v) => format!("{v}+"),
            Bound::Excluded(v) => format!(">{v}"),
        };
    }
    let (upper_version, upper_inclusive) = match upper {
        Bound::Included(v) => (v, true),
        Bound::Excluded(v) => (v, false),
        Bound::Unbounded => unreachable!(),
    };
    let lower_version = match lower {
        Bound::Included(v) | Bound::Excluded(v) => Some(v),
        Bound::Unbounded => None,
    };
    let lower_inclusive = !matches!(lower, Bound::Excluded(_));

    // 2. Single exact version -> "==v".
    if lower_version == Some(upper_version) {
        return format!("=={upper_version}");
    }
    // 3. Both bounds inclusive.
    if lower_inclusive && upper_inclusive {
        return match lower_version {
            Some(lv) => format!("{lv}..{upper_version}"),
            None => format!("<={upper_version}"),
        };
    }
    // 4. The "superset" form: [v, v.next()) prints as just "v".
    if lower_inclusive && !upper_inclusive {
        if let Some(lv) = lower_version {
            if &lv.bump() == upper_version {
                return lv.to_string();
            }
        }
    }
    // 5. General case: lower string followed by upper string.
    let lower_str = match lower {
        Bound::Unbounded => String::new(),
        Bound::Included(v) => format!("{v}+"),
        Bound::Excluded(v) => format!(">{v}"),
    };
    let upper_str = if upper_inclusive {
        format!("<={upper_version}")
    } else {
        format!("<{upper_version}")
    };
    format!("{lower_str}{upper_str}")
}

impl fmt::Display for VersionRange {
    /// Renders in rez's compact range syntax (`3`, `3+<5`, `==1.0`, `2|6+`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for (lower, upper) in self.0.iter() {
            if !first {
                write!(f, "|")?;
            }
            first = false;
            write!(f, "{}", format_segment(lower, upper))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> RerVersion {
        RerVersion::try_from(s).unwrap()
    }

    #[test]
    fn test_any_and_empty() {
        assert!(VersionRange::any().is_any());
        assert!(!VersionRange::any().is_empty());
        assert!(VersionRange::empty().is_empty());
        assert!(!VersionRange::empty().is_any());
        // The parsed empty-string range is the "any" range, per rez.
        assert!(VersionRange::parse("").is_any());
    }

    #[test]
    fn test_intersection_none_when_disjoint() {
        let a = VersionRange::parse("1+<2");
        let b = VersionRange::parse("3+<4");
        assert!(a.intersection(&b).is_none());
        let c = VersionRange::parse("1.5+<3");
        let hit = a.intersection(&c).unwrap();
        assert!(hit.contains(&v("1.6")));
        assert!(!hit.contains(&v("2.0")));
    }

    #[test]
    fn test_inverse_none_only_for_any() {
        assert!(VersionRange::any().inverse().is_none());
        let inv = VersionRange::parse("2+<3").inverse().unwrap();
        assert!(inv.contains(&v("1.0")));
        assert!(inv.contains(&v("4.0")));
        assert!(!inv.contains(&v("2.5")));
    }

    #[test]
    fn test_difference() {
        // a - b = a & ~b
        let a = VersionRange::parse("1+<5");
        let b = VersionRange::parse("2+<3");
        let diff = a.difference(&b).unwrap();
        assert!(diff.contains(&v("1.5")));
        assert!(!diff.contains(&v("2.5")));
        assert!(diff.contains(&v("3.0")));
        // subtracting "any" yields None
        assert!(a.difference(&VersionRange::any()).is_none());
        // subtracting a superset empties the result -> None
        assert!(b.difference(&a).is_none());
    }

    #[test]
    fn test_superset_subset_intersects() {
        let wide = VersionRange::parse("1+<10");
        let narrow = VersionRange::parse("2+<3");
        assert!(wide.issuperset(&narrow));
        assert!(!narrow.issuperset(&wide));
        assert!(narrow.issubset(&wide));
        assert!(wide.intersects(&narrow));
        assert!(!narrow.intersects(&VersionRange::parse("5+<6")));
    }

    #[test]
    fn test_from_version_superset() {
        // rez: the range `3` is the superset of any `3[.X.X...]`.
        let r = VersionRange::from_version(&v("3"));
        assert!(r.contains(&v("3")));
        assert!(r.contains(&v("3.0")));
        assert!(r.contains(&v("3.1.4")));
        assert!(!r.contains(&v("4")));
    }

    #[test]
    fn test_from_versions_and_to_versions() {
        let r = VersionRange::from_versions([v("3"), v("5.1"), v("4")]);
        let mut versions = r.to_versions().unwrap();
        versions.sort();
        assert_eq!(versions, vec![v("3"), v("4"), v("5.1")]);
        // a non-exact range has no exact versions
        assert!(VersionRange::parse("1+<2").to_versions().is_none());
    }

    #[test]
    fn test_display_rez_format() {
        // Compact rez range syntax, not the underlying Ranges debug format.
        assert_eq!(VersionRange::parse("4").to_string(), "4");
        assert_eq!(VersionRange::parse("1.0").to_string(), "1.0");
        assert_eq!(VersionRange::parse("3+<5").to_string(), "3+<5");
        assert_eq!(VersionRange::parse("3+").to_string(), "3+");
        assert_eq!(VersionRange::parse("<3").to_string(), "<3");
        assert_eq!(VersionRange::parse("==1.0.1").to_string(), "==1.0.1");
        assert_eq!(VersionRange::parse("2|5").to_string(), "2|5");
        assert_eq!(VersionRange::any().to_string(), "");
    }

    #[test]
    fn test_span_and_split() {
        let r = VersionRange::parse("2+<4|6+<8");
        let span = r.span();
        assert!(span.contains(&v("5")));
        assert!(span.contains(&v("3")));
        assert!(!span.contains(&v("8")));
        let parts = r.split();
        assert_eq!(parts.len(), 2);
        assert!(parts[0].contains(&v("3")));
        assert!(!parts[0].contains(&v("6")));
        assert!(parts[1].contains(&v("6")));
    }
}
