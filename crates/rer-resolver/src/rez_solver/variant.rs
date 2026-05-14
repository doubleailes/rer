//! Variant data structures, ported from `rez/src/rez/solver.py`
//! (`PackageVariant`, `_PackageEntry`, `_PackageVariantList`,
//! `_PackageVariantSlice`, `PackageVariantCache`).
//!
//! These hold a package family's candidate variants and progressively narrow
//! them as the solve proceeds — by version range (`intersect`), by conflicting
//! requirements (`reduce_by`), and by peeling off the search space (`split`).
//! `extract` pulls out a dependency common to every remaining variant.

use super::context::SolverContext;
use super::requirement::{Requirement, RequirementList};
use rer_version::{RerVersion, VersionRange};
use std::collections::HashSet;
use std::ops::Bound;
use std::rc::Rc;

// ---------------------------------------------------------------------------
// Variant ordering keys (rez's default orderer: `SortedOrder(descending=True)`)
// ---------------------------------------------------------------------------

/// A version slot in a sort key: the empty version (`Min`), a concrete
/// version, or the infinite version (`Inf`) — mirroring rez's `Version()` and
/// `Version.inf` sentinels used by `_LowerBound.min` / `_UpperBound.inf`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum VersionKey {
    Min,
    V(RerVersion),
    Inf,
}

/// Sort key for one contiguous range segment: `((lower_version, incl), (upper_version, incl))`.
type SegmentKey = ((VersionKey, i8), (VersionKey, i8));

/// Sort key for a whole [`VersionRange`] under the default orderer — one
/// `SegmentKey` per contiguous segment. Mirrors `PackageOrder.sort_key` for a
/// `VersionRange` (`rez/src/rez/package_order.py:128`).
type RangeKey = Vec<SegmentKey>;

/// Compute the default-orderer sort key for a requirement's range. `None`
/// (the no-effect `~foo`) yields an empty key — it is never actually compared,
/// because the request position always disambiguates first.
fn range_sort_key(range: Option<&VersionRange>) -> RangeKey {
    let Some(range) = range else {
        return Vec::new();
    };
    range
        .as_ranges()
        .iter()
        .map(|(lower, upper)| {
            // inclusion keys: lower inclusive -2 / exclusive -1; upper
            // inclusive 2 / exclusive 1 (rez package_order.py:134-139).
            let lower_key = match lower {
                Bound::Unbounded => (VersionKey::Min, -2),
                Bound::Included(v) => (VersionKey::V(v.clone()), -2),
                Bound::Excluded(v) => (VersionKey::V(v.clone()), -1),
            };
            let upper_key = match upper {
                Bound::Unbounded => (VersionKey::Inf, 2),
                Bound::Included(v) => (VersionKey::V(v.clone()), 2),
                Bound::Excluded(v) => (VersionKey::V(v.clone()), 1),
            };
            (lower_key, upper_key)
        })
        .collect()
}

/// The variant sort key for `VariantSelectMode.version_priority` (rez's
/// default), from `_PackageEntry.sort`'s `key()` (`solver.py:423`).
///
/// Field order is the comparison order; variants are then sorted with this key
/// in *descending* order ("most correct to consume, to least").
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct VariantKey {
    /// `(-request_index, range_key)` for each top-level request the variant
    /// depends on — prefer variants using higher versions of request-shared
    /// packages, earliest request weighted most.
    requested_key: Vec<(i32, RangeKey)>,
    /// `-len(additional_key)` — prefer fewer extra packages pulled in.
    neg_additional_len: i32,
    /// `(range_key, name)` for the variant's other dependencies — prefer
    /// higher versions of those, then alphabetical by name.
    additional_key: Vec<(RangeKey, String)>,
    /// Final tiebreak; should only matter for otherwise-identical variants.
    index: Option<usize>,
}

// ---------------------------------------------------------------------------
// PackageVariant
// ---------------------------------------------------------------------------

/// One concrete variant of one package version. Mirrors rez's `PackageVariant`.
///
/// Unlike rez, `requires` is computed eagerly — the repository is fully
/// in-memory, so there is no lazy-load cost to defer.
#[derive(Debug)]
pub struct PackageVariant {
    name: String,
    version: RerVersion,
    /// `None` for a package with no variants defined (rendered as `pkg[]`).
    index: Option<usize>,
    /// Base `requires` plus this variant's own requires, merged.
    requires: RequirementList,
    /// Non-conflict dependency family names (`requires_list.names`).
    request_fams: HashSet<String>,
    /// Conflict dependency family names (`requires_list.conflict_names`).
    conflict_request_fams: HashSet<String>,
}

impl PackageVariant {
    fn new(
        name: String,
        version: RerVersion,
        index: Option<usize>,
        requires: RequirementList,
    ) -> Self {
        let request_fams = requires.names().iter().cloned().collect();
        let conflict_request_fams = requires.conflict_names().iter().cloned().collect();
        PackageVariant {
            name,
            version,
            index,
            requires,
            request_fams,
            conflict_request_fams,
        }
    }

    /// Package family name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Concrete version of this variant.
    pub fn version(&self) -> &RerVersion {
        &self.version
    }

    /// Variant index, or `None` if the package defines no variants.
    pub fn index(&self) -> Option<usize> {
        self.index
    }

    /// The variant's merged requirement list.
    pub fn requires(&self) -> &RequirementList {
        &self.requires
    }

    /// This variant's requirement for `pkg_name`, if it has one.
    pub fn get(&self, pkg_name: &str) -> Option<&Requirement> {
        self.requires.get(pkg_name)
    }

    /// Non-conflict dependency family names.
    pub fn request_fams(&self) -> &HashSet<String> {
        &self.request_fams
    }

    /// Conflict dependency family names.
    pub fn conflict_request_fams(&self) -> &HashSet<String> {
        &self.conflict_request_fams
    }

    /// True if the variant's own requires are internally contradictory.
    pub fn has_internal_conflict(&self) -> bool {
        self.requires.conflict().is_some()
    }
}

impl std::fmt::Display for PackageVariant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.index {
            Some(i) => write!(f, "{}-{}[{}]", self.name, self.version, i),
            None => write!(f, "{}-{}[]", self.name, self.version),
        }
    }
}

/// A variant removed from consideration during reduction. Mirrors rez's
/// `Reduction` — kept for failure reporting.
#[derive(Debug, Clone)]
pub struct Reduction {
    /// The reduced variant.
    pub name: String,
    /// The reduced variant's version.
    pub version: RerVersion,
    /// The reduced variant's index.
    pub variant_index: Option<usize>,
    /// The variant's dependency that caused the reduction.
    pub dependency: Requirement,
    /// The request the dependency conflicted with.
    pub conflicting_request: Requirement,
}

impl std::fmt::Display for Reduction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let idx = match self.variant_index {
            Some(i) => i.to_string(),
            None => String::new(),
        };
        write!(
            f,
            "{}-{}[{}] requires {} (conflicts with {})",
            self.name, self.version, idx, self.dependency, self.conflicting_request
        )
    }
}

// ---------------------------------------------------------------------------
// PackageEntry — the variants of one package version
// ---------------------------------------------------------------------------

/// The variants of a single package version. Mirrors rez's `_PackageEntry`.
#[derive(Debug, Clone)]
pub struct PackageEntry {
    version: RerVersion,
    variants: Vec<Rc<PackageVariant>>,
    /// Whether `variants` is in rez's preferred (descending key) order.
    sorted: bool,
}

impl PackageEntry {
    /// Number of variants in this entry.
    pub fn len(&self) -> usize {
        self.variants.len()
    }

    /// True if this entry has no variants.
    pub fn is_empty(&self) -> bool {
        self.variants.is_empty()
    }

    /// The package version.
    pub fn version(&self) -> &RerVersion {
        &self.version
    }

    /// The variants, in current order.
    pub fn variants(&self) -> &[Rc<PackageVariant>] {
        &self.variants
    }

    /// Sort variants from "most correct to consume" to least, per rez's
    /// `_PackageEntry.sort`. Idempotent.
    pub fn sort(&mut self, ctx: &SolverContext) {
        if self.sorted {
            return;
        }
        // Sort by key ascending, then reverse — equivalent to rez's
        // `sort(key=key, reverse=True)` and avoids recomputing keys.
        self.variants
            .sort_by_cached_key(|variant| variant_sort_key(variant, ctx));
        self.variants.reverse();
        self.sorted = true;
    }

    /// Split off the first `nvariants` variants, returning `(entry, rest)`, or
    /// `None` if `nvariants` covers the whole entry. Sorts first.
    pub fn split(
        &mut self,
        ctx: &SolverContext,
        nvariants: usize,
    ) -> Option<(PackageEntry, PackageEntry)> {
        if nvariants >= self.variants.len() {
            return None;
        }
        self.sort(ctx);
        let entry = PackageEntry {
            version: self.version.clone(),
            variants: self.variants[..nvariants].to_vec(),
            sorted: true,
        };
        let next_entry = PackageEntry {
            version: self.version.clone(),
            variants: self.variants[nvariants..].to_vec(),
            sorted: true,
        };
        Some((entry, next_entry))
    }
}

/// Compute a variant's sort key — the `version_priority` `key()` of
/// `_PackageEntry.sort` (`solver.py:423`).
fn variant_sort_key(variant: &PackageVariant, ctx: &SolverContext) -> VariantKey {
    let mut requested_key: Vec<(i32, RangeKey)> = Vec::new();
    let mut names: HashSet<&str> = HashSet::new();

    for (i, request) in ctx.request_list.iter().enumerate() {
        if request.is_conflict() {
            continue;
        }
        if let Some(req) = variant.get(request.name()) {
            requested_key.push((-(i as i32), range_sort_key(req.range())));
            names.insert(req.name());
        }
    }

    let mut additional_key: Vec<(RangeKey, String)> = Vec::new();
    for request in variant.requires().iter() {
        if !request.is_conflict() && !names.contains(request.name()) {
            additional_key.push((range_sort_key(request.range()), request.name().to_string()));
        }
    }

    VariantKey {
        requested_key,
        neg_additional_len: -(additional_key.len() as i32),
        additional_key,
        index: variant.index,
    }
}

// ---------------------------------------------------------------------------
// PackageVariantList — every variant of a family (cached per family)
// ---------------------------------------------------------------------------

/// Every variant of one package family, built once and cached. Mirrors rez's
/// `_PackageVariantList` (minus the lazy-load / package-filter machinery, which
/// the in-memory repo makes unnecessary).
#[derive(Debug)]
pub struct PackageVariantList {
    /// `(version, variants)` for every version of the family, version-sorted
    /// ascending for deterministic iteration.
    entries: Vec<(RerVersion, Vec<Rc<PackageVariant>>)>,
}

impl PackageVariantList {
    /// Build the variant list for a family, or `None` if the family is absent
    /// from the repository.
    pub fn new(ctx: &SolverContext, package_name: &str) -> Option<Self> {
        let versions = ctx.repo.get(package_name)?;
        let mut entries: Vec<(RerVersion, Vec<Rc<PackageVariant>>)> = Vec::new();

        for (version_str, data) in versions {
            let version = RerVersion::try_from(version_str.as_str())
                .unwrap_or_else(|e| panic!("invalid version '{version_str}': {e:?}"));
            let variants = build_variants(package_name, &version, data);
            entries.push((version, variants));
        }
        // Deterministic base order; `_PackageVariantSlice::sort_versions`
        // re-sorts (descending) when ordering actually matters.
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        Some(PackageVariantList { entries })
    }

    /// The `_PackageEntry` list for every version that falls within `range`,
    /// or `None` if none do. Mirrors `_PackageVariantList.get_intersection`.
    pub fn get_intersection(&self, range: &VersionRange) -> Option<Vec<PackageEntry>> {
        let entries: Vec<PackageEntry> = self
            .entries
            .iter()
            .filter(|(version, _)| range.contains(version))
            .map(|(version, variants)| PackageEntry {
                version: version.clone(),
                variants: variants.clone(),
                sorted: false,
            })
            .collect();
        if entries.is_empty() {
            None
        } else {
            Some(entries)
        }
    }
}

/// Build the `PackageVariant`s for one package version from its `PackageData`.
///
/// A package with no `variants` has a single variant with index `None` whose
/// requires are just the base `requires`; otherwise each variant `i` has
/// `requires = base requires ++ variants[i]`.
fn build_variants(
    name: &str,
    version: &RerVersion,
    data: &crate::PackageData,
) -> Vec<Rc<PackageVariant>> {
    let parse_all = |reqs: &[String]| -> RequirementList {
        RequirementList::new(reqs.iter().map(|s| Requirement::parse(s)).collect())
    };

    if data.variants.is_empty() {
        let requires = parse_all(&data.requires);
        vec![Rc::new(PackageVariant::new(
            name.to_string(),
            version.clone(),
            None,
            requires,
        ))]
    } else {
        data.variants
            .iter()
            .enumerate()
            .map(|(i, variant_reqs)| {
                let mut all = data.requires.clone();
                all.extend_from_slice(variant_reqs);
                let requires = parse_all(&all);
                Rc::new(PackageVariant::new(
                    name.to_string(),
                    version.clone(),
                    Some(i),
                    requires,
                ))
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// PackageVariantSlice — a working subset of a family's variants
// ---------------------------------------------------------------------------

/// Result of [`PackageVariantSlice::intersect`].
pub enum SliceIntersect {
    /// The range removed nothing — the slice is unchanged.
    Unchanged,
    /// The range narrowed the slice.
    Narrowed(PackageVariantSlice),
    /// Every variant fell outside the range.
    Empty,
}

/// Result of [`PackageVariantSlice::reduce_by`].
pub enum SliceReduce {
    /// Nothing was reduced — the slice is unchanged.
    Unchanged,
    /// Some variants were reduced away; the rest remain.
    Reduced(PackageVariantSlice),
    /// Every variant was reduced away.
    Empty,
}

/// A working subset of one family's variants, with dependency-related info.
/// Mirrors rez's `_PackageVariantSlice`.
#[derive(Debug, Clone)]
pub struct PackageVariantSlice {
    ctx: Rc<SolverContext>,
    package_name: String,
    entries: Vec<PackageEntry>,
    /// Families already extracted from this slice as common requirements.
    extracted_fams: HashSet<String>,
    /// Whether `entries` is version-sorted (descending).
    sorted: bool,
}

impl PackageVariantSlice {
    /// Build a slice over the given entries.
    pub fn new(ctx: Rc<SolverContext>, package_name: String, entries: Vec<PackageEntry>) -> Self {
        PackageVariantSlice {
            ctx,
            package_name,
            entries,
            extracted_fams: HashSet::new(),
            sorted: false,
        }
    }

    /// The package family name.
    pub fn package_name(&self) -> &str {
        &self.package_name
    }

    /// Total number of variants across all versions.
    pub fn len(&self) -> usize {
        self.entries.iter().map(PackageEntry::len).sum()
    }

    /// True if the slice has no variants (should not normally occur).
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The version range spanned by the slice's versions.
    pub fn range(&self) -> VersionRange {
        VersionRange::from_versions(self.entries.iter().map(|e| e.version.clone()))
    }

    /// Iterate every variant in the slice.
    pub fn iter_variants(&self) -> impl Iterator<Item = &Rc<PackageVariant>> {
        self.entries.iter().flat_map(|e| e.variants.iter())
    }

    /// The first variant of the first (sorted) entry. Sorts that entry.
    pub fn first_variant(&mut self) -> Rc<PackageVariant> {
        let ctx = Rc::clone(&self.ctx);
        let entry = &mut self.entries[0];
        entry.sort(&ctx);
        Rc::clone(&entry.variants[0])
    }

    /// Families every variant depends on (non-conflict). Mirrors `common_fams`.
    pub fn common_fams(&self) -> HashSet<String> {
        let mut iter = self.iter_variants();
        let Some(first) = iter.next() else {
            return HashSet::new();
        };
        let mut common = first.request_fams().clone();
        for variant in iter {
            common.retain(|f| variant.request_fams().contains(f));
        }
        common
    }

    /// Every family any variant mentions, conflict or not. Mirrors `fam_requires`.
    pub fn fam_requires(&self) -> HashSet<String> {
        let mut all = HashSet::new();
        for variant in self.iter_variants() {
            all.extend(variant.request_fams().iter().cloned());
            all.extend(variant.conflict_request_fams().iter().cloned());
        }
        all
    }

    /// True if a common dependency family remains to be extracted.
    pub fn extractable(&self) -> bool {
        !self.common_fams().is_subset(&self.extracted_fams)
    }

    /// Remove variants whose version falls outside `range`.
    pub fn intersect(&self, range: &VersionRange) -> SliceIntersect {
        if range.is_any() {
            return SliceIntersect::Unchanged;
        }
        let entries: Vec<PackageEntry> = self
            .entries
            .iter()
            .filter(|e| range.contains(&e.version))
            .cloned()
            .collect();
        if entries.is_empty() {
            SliceIntersect::Empty
        } else if entries.len() < self.entries.len() {
            SliceIntersect::Narrowed(self.copy_with_entries(entries))
        } else {
            SliceIntersect::Unchanged
        }
    }

    /// Remove variants whose dependencies conflict with `package_request`.
    pub fn reduce_by(&self, package_request: &Requirement) -> (SliceReduce, Vec<Reduction>) {
        // Nothing to reduce against: a no-effect `~foo`, or a family that no
        // variant here depends on.
        if package_request.range().is_none()
            || !self.fam_requires().contains(package_request.name())
        {
            return (SliceReduce::Unchanged, Vec::new());
        }

        let mut new_entries: Vec<PackageEntry> = Vec::new();
        let mut reductions: Vec<Reduction> = Vec::new();

        for entry in &self.entries {
            let mut kept: Vec<Rc<PackageVariant>> = Vec::new();
            for variant in &entry.variants {
                match variant.get(package_request.name()) {
                    Some(req) if req.conflicts_with(package_request) => {
                        reductions.push(Reduction {
                            name: variant.name().to_string(),
                            version: variant.version().clone(),
                            variant_index: variant.index(),
                            dependency: req.clone(),
                            conflicting_request: package_request.clone(),
                        });
                    }
                    _ => kept.push(Rc::clone(variant)),
                }
            }
            if kept.is_empty() {
                continue; // whole version dropped
            }
            if kept.len() < entry.variants.len() {
                new_entries.push(PackageEntry {
                    version: entry.version.clone(),
                    variants: kept,
                    sorted: entry.sorted,
                });
            } else {
                new_entries.push(entry.clone());
            }
        }

        if new_entries.is_empty() {
            (SliceReduce::Empty, reductions)
        } else if !reductions.is_empty() {
            (
                SliceReduce::Reduced(self.copy_with_entries(new_entries)),
                reductions,
            )
        } else {
            (SliceReduce::Unchanged, Vec::new())
        }
    }

    /// Extract a dependency common to every variant, returning the narrowed
    /// slice and the extracted requirement. `None` if nothing is extractable.
    ///
    /// Conflict dependencies are never extracted — they are resolved only via
    /// reduction.
    pub fn extract(&self) -> Option<(PackageVariantSlice, Requirement)> {
        if !self.extractable() {
            return None;
        }
        let extractable: HashSet<String> = self
            .common_fams()
            .difference(&self.extracted_fams)
            .cloned()
            .collect();
        // Sorted pick — required for deterministic solves.
        let fam = extractable
            .iter()
            .min()
            .expect("extractable is non-empty")
            .clone();

        // Union of every variant's range for `fam`.
        let mut range: Option<VersionRange> = None;
        for variant in self.iter_variants() {
            let req = variant
                .get(&fam)
                .expect("every variant depends on a common family");
            let req_range = req
                .range()
                .expect("a non-conflict requirement always has a range");
            range = Some(match range {
                None => req_range.clone(),
                Some(acc) => acc.union(req_range),
            });
        }

        let mut slice = self.clone(); // keeps extracted_fams (rez's `copy.copy`)
        slice.extracted_fams.insert(fam.clone());
        let common_req = Requirement::construct(fam, range);
        Some((slice, common_req))
    }

    /// Split the slice into a preferred half and the remainder. The preferred
    /// half is constructed so its variants share a dependency family (so it is
    /// `extractable` again — i.e. the solve can make progress).
    pub fn split(&mut self) -> (PackageVariantSlice, PackageVariantSlice) {
        self.sort_versions();

        // Decide whether to look for the first variant without a common
        // dependency, or just peel off the single best variant.
        let mut fams: HashSet<String> = if self.len() > 2 {
            let first = self.first_variant();
            first
                .request_fams()
                .difference(&self.extracted_fams)
                .cloned()
                .collect()
        } else {
            HashSet::new()
        };

        if fams.is_empty() {
            let ctx = Rc::clone(&self.ctx);
            self.entries[0].sort(&ctx);
            return self.split_at(0, 1);
        }

        // Find the split point: the first variant sharing no dependency family
        // with every variant before it.
        let ctx = Rc::clone(&self.ctx);
        let mut prev: Option<(usize, usize)> = None;
        for i in 0..self.entries.len() {
            self.entries[i].sort(&ctx);
            for j in 0..self.entries[i].variants.len() {
                let variant_fams = self.entries[i].variants[j].request_fams();
                fams.retain(|f| variant_fams.contains(f));
                if fams.is_empty() {
                    let (pi, pj) = prev.expect("split point cannot be the very first variant");
                    return self.split_at(pi, pj);
                }
                prev = Some((i, j + 1));
            }
        }
        unreachable!(
            "split() called on a slice with a common dependency family — \
             it should have been extracted instead"
        );
    }

    /// Perform a split at entry `i_entry`, peeling off `n_variants` variants.
    fn split_at(
        &mut self,
        i_entry: usize,
        n_variants: usize,
    ) -> (PackageVariantSlice, PackageVariantSlice) {
        let ctx = Rc::clone(&self.ctx);
        let split = self.entries[i_entry].split(&ctx, n_variants);

        let (entries, next_entries) = match split {
            Some((entry, next_entry)) => {
                let mut entries = self.entries[..i_entry].to_vec();
                entries.push(entry);
                let mut next_entries = vec![next_entry];
                next_entries.extend_from_slice(&self.entries[i_entry + 1..]);
                (entries, next_entries)
            }
            None => (
                self.entries[..=i_entry].to_vec(),
                self.entries[i_entry + 1..].to_vec(),
            ),
        };

        (
            self.copy_with_entries(entries),
            self.copy_with_entries(next_entries),
        )
    }

    /// Sort entries by version, descending. Idempotent.
    pub fn sort_versions(&mut self) {
        if self.sorted {
            return;
        }
        self.entries.sort_by(|a, b| b.version.cmp(&a.version));
        self.sorted = true;
    }

    /// rez's `_copy`: a new slice over `new_entries` carrying the `sorted`
    /// flag but with a *fresh* (empty) `extracted_fams`.
    fn copy_with_entries(&self, new_entries: Vec<PackageEntry>) -> PackageVariantSlice {
        PackageVariantSlice {
            ctx: Rc::clone(&self.ctx),
            package_name: self.package_name.clone(),
            entries: new_entries,
            extracted_fams: HashSet::new(),
            sorted: self.sorted,
        }
    }
}

impl std::fmt::Display for PackageVariantSlice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}[{} variants]", self.package_name, self.len())
    }
}

// ---------------------------------------------------------------------------
// PackageVariantCache
// ---------------------------------------------------------------------------

/// Caches one [`PackageVariantList`] per family so variants are built once.
/// Mirrors rez's `PackageVariantCache`.
#[derive(Debug, Default)]
pub struct PackageVariantCache {
    /// `family -> Some(list)` if found, `family -> None` if absent.
    variant_lists: std::collections::HashMap<String, Option<Rc<PackageVariantList>>>,
}

impl PackageVariantCache {
    /// A fresh, empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get a slice of `package_name`'s variants intersected with `range`.
    ///
    /// `None` means either the family is absent from the repository or no
    /// version falls within `range` — Phase 4/5 distinguishes the two.
    pub fn get_variant_slice(
        &mut self,
        ctx: &Rc<SolverContext>,
        package_name: &str,
        range: &VersionRange,
    ) -> Option<PackageVariantSlice> {
        let list = self
            .variant_lists
            .entry(package_name.to_string())
            .or_insert_with(|| PackageVariantList::new(ctx, package_name).map(Rc::new))
            .clone()?;

        let entries = list.get_intersection(range)?;
        Some(PackageVariantSlice::new(
            Rc::clone(ctx),
            package_name.to_string(),
            entries,
        ))
    }

    /// True if the family is known to be absent from the repository.
    pub fn family_missing(&mut self, ctx: &Rc<SolverContext>, package_name: &str) -> bool {
        self.variant_lists
            .entry(package_name.to_string())
            .or_insert_with(|| PackageVariantList::new(ctx, package_name).map(Rc::new))
            .is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::super::context::PackageRepo;
    use super::*;
    use crate::PackageData;

    fn pkg(requires: &[&str], variants: &[&[&str]]) -> PackageData {
        PackageData {
            requires: requires.iter().map(|s| s.to_string()).collect(),
            variants: variants
                .iter()
                .map(|v| v.iter().map(|s| s.to_string()).collect())
                .collect(),
        }
    }

    fn repo(entries: Vec<(&str, Vec<(&str, PackageData)>)>) -> PackageRepo {
        entries
            .into_iter()
            .map(|(name, versions)| {
                (
                    name.to_string(),
                    versions
                        .into_iter()
                        .map(|(v, d)| (v.to_string(), d))
                        .collect(),
                )
            })
            .collect()
    }

    fn ctx_with(repo: PackageRepo, requests: &[&str]) -> Rc<SolverContext> {
        let request_list =
            RequirementList::new(requests.iter().map(|s| Requirement::parse(s)).collect());
        Rc::new(SolverContext::new(repo, request_list))
    }

    #[test]
    fn test_build_variants_no_variants() {
        let r = repo(vec![("foo", vec![("1.0", pkg(&["bar-2"], &[]))])]);
        let ctx = ctx_with(r, &["foo"]);
        let list = PackageVariantList::new(&ctx, "foo").unwrap();
        let entries = list.get_intersection(&VersionRange::any()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].len(), 1);
        let v = &entries[0].variants()[0];
        assert_eq!(v.index(), None);
        assert!(v.request_fams().contains("bar"));
    }

    #[test]
    fn test_build_variants_with_variants() {
        let r = repo(vec![(
            "foo",
            vec![("1.0", pkg(&["base-1"], &[&["maya-2024"], &["maya-2025"]]))],
        )]);
        let ctx = ctx_with(r, &["foo"]);
        let list = PackageVariantList::new(&ctx, "foo").unwrap();
        let entries = list.get_intersection(&VersionRange::any()).unwrap();
        assert_eq!(entries[0].len(), 2);
        let v0 = &entries[0].variants()[0];
        assert_eq!(v0.index(), Some(0));
        // variant 0 has base + its own requires
        assert!(v0.request_fams().contains("base"));
        assert!(v0.request_fams().contains("maya"));
    }

    #[test]
    fn test_get_intersection_filters_by_range() {
        let r = repo(vec![(
            "foo",
            vec![
                ("1.0", pkg(&[], &[])),
                ("2.0", pkg(&[], &[])),
                ("3.0", pkg(&[], &[])),
            ],
        )]);
        let ctx = ctx_with(r, &["foo"]);
        let list = PackageVariantList::new(&ctx, "foo").unwrap();
        let entries = list.get_intersection(&VersionRange::parse("2+")).unwrap();
        let versions: Vec<String> = entries.iter().map(|e| e.version().to_string()).collect();
        assert_eq!(versions, vec!["2.0", "3.0"]);
    }

    #[test]
    fn test_slice_intersect_and_reduce() {
        let r = repo(vec![
            (
                "foo",
                vec![("1.0", pkg(&["bar-1"], &[])), ("2.0", pkg(&["bar-2"], &[]))],
            ),
            ("bar", vec![("1.0", pkg(&[], &[])), ("2.0", pkg(&[], &[]))]),
        ]);
        let ctx = ctx_with(r, &["foo"]);
        let mut cache = PackageVariantCache::new();
        let slice = cache
            .get_variant_slice(&ctx, "foo", &VersionRange::any())
            .unwrap();
        assert_eq!(slice.len(), 2);

        // intersect to just foo-2
        match slice.intersect(&VersionRange::parse("2")) {
            SliceIntersect::Narrowed(s) => assert_eq!(s.len(), 1),
            _ => panic!("expected narrowed"),
        }

        // reduce foo by !bar-1: foo-1.0 (requires bar-1) is reduced away
        let (result, reductions) = slice.reduce_by(&Requirement::parse("!bar-1"));
        match result {
            SliceReduce::Reduced(s) => {
                assert_eq!(s.len(), 1);
                assert_eq!(reductions.len(), 1);
                assert_eq!(reductions[0].version.to_string(), "1.0");
            }
            _ => panic!("expected reduced"),
        }
    }

    #[test]
    fn test_slice_extract_common_dependency() {
        // both versions of foo depend on `bar` -> bar is a common, extractable family
        let r = repo(vec![(
            "foo",
            vec![("1.0", pkg(&["bar-1"], &[])), ("2.0", pkg(&["bar-2"], &[]))],
        )]);
        let ctx = ctx_with(r, &["foo"]);
        let mut cache = PackageVariantCache::new();
        let slice = cache
            .get_variant_slice(&ctx, "foo", &VersionRange::any())
            .unwrap();
        assert!(slice.extractable());
        let (narrowed, req) = slice.extract().expect("bar is extractable");
        assert_eq!(req.name(), "bar");
        // extracted req is the union of bar-1 and bar-2
        assert!(req.range().unwrap().contains(&"1.5".try_into().unwrap()));
        assert!(req.range().unwrap().contains(&"2.5".try_into().unwrap()));
        // once extracted, nothing more is extractable
        assert!(!narrowed.extractable());
    }
}
