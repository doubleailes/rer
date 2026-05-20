//! Variant data structures, ported from `rez/src/rez/solver.py`
//! (`PackageVariant`, `_PackageEntry`, `_PackageVariantList`,
//! `_PackageVariantSlice`, `PackageVariantCache`).
//!
//! These hold a package family's candidate variants and progressively narrow
//! them as the solve proceeds — by version range (`intersect`), by conflicting
//! requirements (`reduce_by`), and by peeling off the search space (`split`).
//! `extract` pulls out a dependency common to every remaining variant.

use super::context::{FamilyMap, SolverContext};
use super::requirement::{Requirement, RequirementList};
use super::Name;
use rer_version::{RerVersion, VersionRange};
use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::{OnceCell, RefCell};
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

/// The variant sort key, ported from `_PackageEntry.sort`'s `key()` in
/// `solver.py:423-455`. The same struct serves both
/// [`VariantSelectMode`](super::context::VariantSelectMode) flavours: only
/// `requested_match_count` differs between them.
///
/// Field order is the comparison order; variants are then sorted with this
/// key in *descending* order ("most correct to consume, to least").
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct VariantKey {
    /// `intersection_priority` only — number of top-level requests this
    /// variant satisfies. Always `0` in `version_priority`, which makes this
    /// field uniform across all variants of the same entry in that mode and
    /// so a no-op for the sort (the secondary keys then drive entirely).
    /// In `intersection_priority`, this is the primary key (`solver.py:449`).
    requested_match_count: i32,
    /// `(-request_index, range_key)` for each top-level request the variant
    /// depends on — prefer variants using higher versions of request-shared
    /// packages, earliest request weighted most.
    requested_key: Vec<(i32, RangeKey)>,
    /// `-len(additional_key)` — prefer fewer extra packages pulled in.
    neg_additional_len: i32,
    /// `(range_key, name)` for the variant's other dependencies — prefer
    /// higher versions of those, then alphabetical by name.
    additional_key: Vec<(RangeKey, Name)>,
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
    name: Name,
    version: RerVersion,
    /// `None` for a package with no variants defined (rendered as `pkg[]`).
    index: Option<usize>,
    /// Base `requires` plus this variant's own requires, merged.
    requires: RequirementList,
    /// Non-conflict dependency family names (`requires_list.names`).
    request_fams: FxHashSet<Name>,
    /// Conflict dependency family names (`requires_list.conflict_names`).
    conflict_request_fams: FxHashSet<Name>,
}

impl PackageVariant {
    fn new(
        name: Name,
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
    pub fn request_fams(&self) -> &FxHashSet<Name> {
        &self.request_fams
    }

    /// Conflict dependency family names.
    pub fn conflict_request_fams(&self) -> &FxHashSet<Name> {
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
    pub name: Name,
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
    /// This version's rank in the family's preference order — 0 is the
    /// most-preferred version. Computed once per family in
    /// [`PackageVariantList::new`] (default: version-descending; or via
    /// the pluggable [`FamilyOrderer`](super::context::FamilyOrderer)).
    /// `PackageVariantSlice::sort_versions` sorts by this.
    order_rank: usize,
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
            order_rank: self.order_rank,
        };
        let next_entry = PackageEntry {
            version: self.version.clone(),
            variants: self.variants[nvariants..].to_vec(),
            sorted: true,
            order_rank: self.order_rank,
        };
        Some((entry, next_entry))
    }
}

/// Compute a variant's sort key — the `key()` of `_PackageEntry.sort`
/// (`solver.py:423-455`). Shared between `version_priority` and
/// `intersection_priority`: the only difference is that
/// `intersection_priority` puts the request-match *count* in front of the
/// rest of the key (`solver.py:449`), and we encode that by setting
/// `requested_match_count` to the real count in that mode and to `0` (a
/// constant across all variants of the same entry) in `version_priority`.
fn variant_sort_key(variant: &PackageVariant, ctx: &SolverContext) -> VariantKey {
    let mut requested_key: Vec<(i32, RangeKey)> = Vec::new();
    let mut names: FxHashSet<&str> = FxHashSet::default();

    for (i, request) in ctx.request_list.iter().enumerate() {
        if request.is_conflict() {
            continue;
        }
        if let Some(req) = variant.get(request.name()) {
            requested_key.push((-(i as i32), range_sort_key(req.range())));
            names.insert(req.name());
        }
    }

    let mut additional_key: Vec<(RangeKey, Name)> = Vec::new();
    for request in variant.requires().iter() {
        if !request.is_conflict() && !names.contains(request.name()) {
            additional_key.push((range_sort_key(request.range()), Name::from(request.name())));
        }
    }

    let requested_match_count = match ctx.variant_select_mode {
        super::context::VariantSelectMode::VersionPriority => 0,
        super::context::VariantSelectMode::IntersectionPriority => requested_key.len() as i32,
    };

    VariantKey {
        requested_match_count,
        requested_key,
        neg_additional_len: -(additional_key.len() as i32),
        additional_key,
        index: variant.index,
    }
}

// ---------------------------------------------------------------------------
// PackageVariantList — every variant of a family (cached per family)
// ---------------------------------------------------------------------------

/// Turn a [`FamilyOrderer`](super::context::FamilyOrderer)'s output into a
/// `version_str -> rank` map (0 = most preferred). This is the single
/// enforcement point for the orderer's advisory contract — a misbehaving
/// orderer can never panic the solver:
///
/// - A version named in `ordered` keeps its **first-seen** position as its
///   rank (a duplicate in `ordered` uses the first occurrence).
/// - A version in `input` the orderer **omitted** sinks to the bottom — it
///   gets a rank worse than every named version, ties broken by `input`
///   order (deterministic).
/// - A version in `ordered` **not in `input`** is ignored.
///
/// Every `input` version ends up with exactly one rank.
fn build_rank_map(input: &[&str], ordered: Vec<String>) -> FxHashMap<String, usize> {
    let input_set: FxHashSet<&str> = input.iter().copied().collect();
    let mut ranks: FxHashMap<String, usize> = FxHashMap::default();
    ranks.reserve(input.len());
    let mut next_rank = 0usize;
    // Named-by-the-orderer versions, in the orderer's order.
    for v in &ordered {
        if input_set.contains(v.as_str()) && !ranks.contains_key(v.as_str()) {
            ranks.insert(v.clone(), next_rank);
            next_rank += 1;
        }
    }
    // Omitted versions sink to the bottom, keeping input order.
    for v in input {
        if !ranks.contains_key(*v) {
            ranks.insert((*v).to_string(), next_rank);
            next_rank += 1;
        }
    }
    ranks
}

/// One version of a family, whose `PackageEntry` (parsed requirements) is
/// materialised lazily on first access.
#[derive(Debug)]
struct LazyEntry {
    version: RerVersion,
    /// The repository key for this version, used to look its data back up.
    version_str: String,
    /// Preference rank for this version — 0 is most preferred. Stamped
    /// onto the `PackageEntry` built from this `LazyEntry`.
    order_rank: usize,
    /// `None` until the version is first touched by a range intersection;
    /// thereafter the built `Rc<PackageEntry>` — unsorted, and shared (by
    /// `Rc`) with every slice that intersects this version.
    entry: RefCell<Option<Rc<PackageEntry>>>,
}

/// Every version of one package family, built once and cached.
///
/// Mirrors rez's `_PackageVariantList`: a version's variants — which means
/// *parsing all its requirement strings* — are materialised lazily, only when
/// some range actually intersects that version. This avoids parsing the
/// requirements of versions no resolve ever looks at (the dominant cost before
/// this was added).
#[derive(Debug)]
pub struct PackageVariantList {
    package_name: Name,
    /// The family's `version_str -> PackageData` map. Held as `Rc` so the
    /// repo and the variant list share storage cheaply; this also lets the
    /// repo discard the map after handing it out (relevant for the
    /// loader-backed case).
    versions: Rc<FamilyMap>,
    /// One entry per version, version-sorted ascending for deterministic
    /// iteration; `_PackageVariantSlice::sort_versions` re-sorts when ordering
    /// actually matters.
    entries: Vec<LazyEntry>,
}

impl PackageVariantList {
    /// Build the (lazy) variant list for a family, or `None` if the family is
    /// absent from the repository. Only version strings are parsed here — the
    /// requirement strings are parsed on demand by [`Self::get_intersection`].
    ///
    /// `hint` is the current solver range constraint, forwarded to the repo's
    /// loader so the shim can pre-filter versions (issue #92). `None` means
    /// "unconstrained — every version".
    pub fn new(
        ctx: &SolverContext,
        package_name: &str,
        hint: Option<&VersionRange>,
    ) -> Option<Self> {
        let versions = ctx.repo.get_family(package_name, hint)?;
        let mut entries: Vec<LazyEntry> = versions
            .keys()
            .map(|version_str| {
                let version = RerVersion::try_from(version_str.as_str())
                    .unwrap_or_else(|e| panic!("invalid version '{version_str}': {e:?}"));
                LazyEntry {
                    version,
                    version_str: version_str.clone(),
                    order_rank: 0, // assigned below
                    entry: RefCell::new(None),
                }
            })
            .collect();
        entries.sort_by(|a, b| a.version.cmp(&b.version));

        // Assign preference ranks (`order_rank` 0 = most preferred). The
        // orderer is (re-)invoked on every (re)build of this list — including
        // the widened-range reload path of issue #92 — so ranks are always
        // recomputed wholesale; no stale ranks survive.
        match &ctx.package_order {
            None => {
                // Default — rez's `SortedOrder(descending=True)`: highest
                // version most preferred. `entries` is ascending, so the
                // rank is the reversed index.
                let n = entries.len();
                for (i, e) in entries.iter_mut().enumerate() {
                    e.order_rank = n - 1 - i;
                }
            }
            Some(orderer) => {
                let version_strs: Vec<&str> =
                    entries.iter().map(|e| e.version_str.as_str()).collect();
                let ordered = orderer(package_name, &version_strs);
                let ranks = build_rank_map(&version_strs, ordered);
                for e in entries.iter_mut() {
                    e.order_rank = ranks[e.version_str.as_str()];
                }
            }
        }

        Some(PackageVariantList {
            package_name: Name::from(package_name),
            versions,
            entries,
        })
    }

    /// The `_PackageEntry` list for every version within `range`, or `None` if
    /// none qualify. Each qualifying version's `PackageEntry` is built on first
    /// use and memoised as a shared `Rc`, so a version is parsed at most once
    /// per solve and unchanged entries are shared across slices.
    pub fn get_intersection(&self, range: &VersionRange) -> Option<Vec<Rc<PackageEntry>>> {
        let mut out: Vec<Rc<PackageEntry>> = Vec::new();
        for lazy in &self.entries {
            if !range.contains(&lazy.version) {
                continue;
            }
            let mut slot = lazy.entry.borrow_mut();
            let built = slot.get_or_insert_with(|| {
                let data = &self.versions[&lazy.version_str];
                Rc::new(PackageEntry {
                    version: lazy.version.clone(),
                    variants: build_variants(&self.package_name, &lazy.version, data),
                    sorted: false,
                    order_rank: lazy.order_rank,
                })
            });
            out.push(Rc::clone(built));
        }
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    }
}

/// Build the `PackageVariant`s for one package version from its `PackageData`.
///
/// A package with no `variants` has a single variant with index `None` whose
/// requires are just the base `requires`; otherwise each variant `i` has
/// `requires = base requires ++ variants[i]`.
fn build_variants(
    name: &Name,
    version: &RerVersion,
    data: &crate::PackageData,
) -> Vec<Rc<PackageVariant>> {
    let parse_all = |reqs: &[String]| -> RequirementList {
        RequirementList::new(reqs.iter().map(|s| Requirement::parse(s)).collect())
    };

    if data.variants.is_empty() {
        let requires = parse_all(&data.requires);
        vec![Rc::new(PackageVariant::new(
            Name::clone(name),
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
                    Name::clone(name),
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
///
/// `len`, `range`, `common_fams` and `fam_requires` are derived from `entries`
/// and cached on first use — they are hot paths during a solve, and rez caches
/// them too. The caches stay valid across a plain `Clone` (the entries are
/// identical) but `copy_with_entries` starts fresh, since the entries differ.
#[derive(Debug, Clone)]
pub struct PackageVariantSlice {
    ctx: Rc<SolverContext>,
    package_name: Name,
    /// Entries are `Rc`-shared: `intersect`/`reduce_by`/`split` keep most
    /// entries unchanged, so passing them through is a refcount bump rather
    /// than a deep `Vec<Rc<PackageVariant>>` clone. `sort`/`split` of an entry
    /// go through `Rc::make_mut` (copy-on-write).
    entries: Vec<Rc<PackageEntry>>,
    /// Families already extracted from this slice as common requirements.
    extracted_fams: FxHashSet<Name>,
    /// Whether `entries` is in preference order (`sort_versions` applied).
    sorted: bool,
    // Lazily-computed, entries-derived caches.
    len_cache: OnceCell<usize>,
    range_cache: OnceCell<VersionRange>,
    common_fams_cache: OnceCell<FxHashSet<Name>>,
    fam_requires_cache: OnceCell<FxHashSet<Name>>,
}

impl PackageVariantSlice {
    /// Build a slice over the given entries.
    pub fn new(ctx: Rc<SolverContext>, package_name: Name, entries: Vec<Rc<PackageEntry>>) -> Self {
        PackageVariantSlice {
            ctx,
            package_name,
            entries,
            extracted_fams: FxHashSet::default(),
            sorted: false,
            len_cache: OnceCell::new(),
            range_cache: OnceCell::new(),
            common_fams_cache: OnceCell::new(),
            fam_requires_cache: OnceCell::new(),
        }
    }

    /// The package family name.
    pub fn package_name(&self) -> &str {
        &self.package_name
    }

    /// Total number of variants across all versions.
    pub fn len(&self) -> usize {
        *self
            .len_cache
            .get_or_init(|| self.entries.iter().map(|e| e.len()).sum())
    }

    /// True if the slice has no variants (should not normally occur).
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The version range spanned by the slice's versions.
    pub fn range(&self) -> VersionRange {
        self.range_cache
            .get_or_init(|| {
                VersionRange::from_versions(self.entries.iter().map(|e| e.version.clone()))
            })
            .clone()
    }

    /// Iterate every variant in the slice.
    pub fn iter_variants(&self) -> impl Iterator<Item = &Rc<PackageVariant>> {
        self.entries.iter().flat_map(|e| e.variants.iter())
    }

    /// The first variant of the first (sorted) entry. Sorts that entry
    /// (copy-on-write if it is shared with another slice).
    pub fn first_variant(&mut self) -> Rc<PackageVariant> {
        let ctx = Rc::clone(&self.ctx);
        let entry = Rc::make_mut(&mut self.entries[0]);
        entry.sort(&ctx);
        Rc::clone(&entry.variants[0])
    }

    /// Families every variant depends on (non-conflict). Mirrors `common_fams`.
    pub fn common_fams(&self) -> &FxHashSet<Name> {
        self.common_fams_cache.get_or_init(|| {
            let mut iter = self.iter_variants();
            let Some(first) = iter.next() else {
                return FxHashSet::default();
            };
            let mut common = first.request_fams().clone();
            for variant in iter {
                common.retain(|f| variant.request_fams().contains(f));
            }
            common
        })
    }

    /// Every family any variant mentions, conflict or not. Mirrors `fam_requires`.
    pub fn fam_requires(&self) -> &FxHashSet<Name> {
        self.fam_requires_cache.get_or_init(|| {
            let mut all = FxHashSet::default();
            for variant in self.iter_variants() {
                all.extend(variant.request_fams().iter().cloned());
                all.extend(variant.conflict_request_fams().iter().cloned());
            }
            all
        })
    }

    /// True if a common dependency family remains to be extracted.
    ///
    /// `extracted_fams` is only ever populated by inserting an element of
    /// `common_fams.difference(extracted_fams)` (in [`Self::extract`]), and
    /// `copy_with_entries` resets it to empty. So `extracted_fams ⊆
    /// common_fams` always holds; under that invariant
    /// `common_fams.is_subset(extracted_fams)` is equivalent to
    /// `common_fams.len() == extracted_fams.len()`.
    ///
    /// Profile data (post-#66/#67/#68/#70): the equivalent
    /// `HashSet::is_subset` was 11.8 % of inclusive cycles — nearly half of
    /// `extract`'s total. Replacing it with the length compare is O(1) and
    /// shrinks the early-return path of every `extract` call to ~nothing.
    pub fn extractable(&self) -> bool {
        self.common_fams().len() > self.extracted_fams.len()
    }

    /// Remove variants whose version falls outside `range`.
    pub fn intersect(&self, range: &VersionRange) -> SliceIntersect {
        if range.is_any() {
            return SliceIntersect::Unchanged;
        }
        // `.cloned()` on `&Rc<PackageEntry>` is a refcount bump.
        let entries: Vec<Rc<PackageEntry>> = self
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

        let mut new_entries: Vec<Rc<PackageEntry>> = Vec::new();
        let mut reductions: Vec<Reduction> = Vec::new();
        // Per-call cache: many variants share the same requirement for this
        // family, and `conflicts_with` is a comparatively expensive range op.
        // Keyed by `&Requirement` — the map hashes/compares by value, so
        // equal-but-distinct requirements share a cache entry without cloning.
        let mut conflict_tests: FxHashMap<&Requirement, bool> = FxHashMap::default();

        for entry in &self.entries {
            let mut kept: Vec<Rc<PackageVariant>> = Vec::new();
            for variant in &entry.variants {
                match variant.get(package_request.name()) {
                    Some(req) => {
                        let conflicts = *conflict_tests
                            .entry(req)
                            .or_insert_with(|| req.conflicts_with(package_request));
                        if conflicts {
                            reductions.push(Reduction {
                                name: Name::clone(&variant.name),
                                version: variant.version().clone(),
                                variant_index: variant.index(),
                                dependency: req.clone(),
                                conflicting_request: package_request.clone(),
                            });
                        } else {
                            kept.push(Rc::clone(variant));
                        }
                    }
                    None => kept.push(Rc::clone(variant)),
                }
            }
            if kept.is_empty() {
                continue; // whole version dropped
            }
            if kept.len() < entry.variants.len() {
                new_entries.push(Rc::new(PackageEntry {
                    version: entry.version.clone(),
                    variants: kept,
                    sorted: entry.sorted,
                    order_rank: entry.order_rank,
                }));
            } else {
                // Unchanged — share the entry rather than deep-cloning it.
                new_entries.push(Rc::clone(entry));
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
        let extractable: FxHashSet<Name> = self
            .common_fams()
            .difference(&self.extracted_fams)
            .cloned()
            .collect();
        // Sorted pick — required for deterministic solves.
        let fam: Name = extractable
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
        let mut fams: FxHashSet<Name> = if self.len() > 2 {
            let first = self.first_variant();
            first
                .request_fams()
                .difference(&self.extracted_fams)
                .cloned()
                .collect()
        } else {
            FxHashSet::default()
        };

        if fams.is_empty() {
            let ctx = Rc::clone(&self.ctx);
            Rc::make_mut(&mut self.entries[0]).sort(&ctx);
            return self.split_at(0, 1);
        }

        // Find the split point: the first variant sharing no dependency family
        // with every variant before it.
        let ctx = Rc::clone(&self.ctx);
        let mut prev: Option<(usize, usize)> = None;
        for i in 0..self.entries.len() {
            Rc::make_mut(&mut self.entries[i]).sort(&ctx);
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
        let split = Rc::make_mut(&mut self.entries[i_entry]).split(&ctx, n_variants);

        let (entries, next_entries) = match split {
            Some((entry, next_entry)) => {
                let mut entries = self.entries[..i_entry].to_vec();
                entries.push(Rc::new(entry));
                let mut next_entries = vec![Rc::new(next_entry)];
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

    /// Sort entries into the family's preference order — most-preferred
    /// version first. By default that is version-descending (rez's
    /// `SortedOrder`); with a pluggable
    /// [`FamilyOrderer`](super::context::FamilyOrderer) it is whatever
    /// order that orderer produced. The preference is baked into each
    /// entry's `order_rank` (0 = most preferred) by
    /// [`PackageVariantList::new`]. Idempotent.
    pub fn sort_versions(&mut self) {
        if self.sorted {
            return;
        }
        self.entries.sort_by(|a, b| a.order_rank.cmp(&b.order_rank));
        self.sorted = true;
    }

    /// rez's `_copy`: a new slice over `new_entries` carrying the `sorted`
    /// flag but with a *fresh* (empty) `extracted_fams` and fresh caches (the
    /// entries differ, so the derived caches must be recomputed).
    fn copy_with_entries(&self, new_entries: Vec<Rc<PackageEntry>>) -> PackageVariantSlice {
        PackageVariantSlice {
            ctx: Rc::clone(&self.ctx),
            package_name: self.package_name.clone(),
            entries: new_entries,
            extracted_fams: FxHashSet::default(),
            sorted: self.sorted,
            len_cache: OnceCell::new(),
            range_cache: OnceCell::new(),
            common_fams_cache: OnceCell::new(),
            fam_requires_cache: OnceCell::new(),
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
    variant_lists: FxHashMap<Name, Option<Rc<PackageVariantList>>>,
}

impl PackageVariantCache {
    /// A fresh, empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get a slice of `package_name`'s variants intersected with `range`.
    ///
    /// `None` means either the family is absent from the repository or no
    /// version falls within `range` — Phase 4/5 distinguishes the two via
    /// [`Self::family_missing`].
    ///
    /// `range` is also forwarded to the repo's loader as the hint, so a
    /// `load_family` callback can pre-filter to versions intersecting it
    /// (issue #92). If a later request asks for a wider range than the
    /// cached load covered, both the repo and this cache transparently
    /// reload.
    pub fn get_variant_slice(
        &mut self,
        ctx: &Rc<SolverContext>,
        package_name: &str,
        range: &VersionRange,
    ) -> Option<PackageVariantSlice> {
        let list = self.get_or_build(ctx, package_name, Some(range))?;
        let entries = list.get_intersection(range)?;
        Some(PackageVariantSlice::new(
            Rc::clone(ctx),
            Name::clone(&list.package_name),
            entries,
        ))
    }

    /// True if the family is known to be absent from the repository.
    /// Asks with `None` hint to force a full load — definitive absence
    /// can't be answered from a range-bounded cached result.
    pub fn family_missing(&mut self, ctx: &Rc<SolverContext>, package_name: &str) -> bool {
        self.get_or_build(ctx, package_name, None).is_none()
    }

    /// Look up the cached `PackageVariantList` for `package_name`, building it
    /// on first access. Lookup is by `&str` (`Borrow<str>` on `Rc<str>`), so a
    /// cache hit avoids allocating a fresh `Name` key.
    ///
    /// The cached list is invalidated and rebuilt if the underlying family
    /// map in [`PackageRepo`] has been reloaded (detected via `Rc::ptr_eq`
    /// on the family map). That happens when a later request needs a
    /// wider range than the previous load covered — see the issue #92
    /// backtrack-widen path in [`PackageRepo::get_family`].
    fn get_or_build(
        &mut self,
        ctx: &Rc<SolverContext>,
        package_name: &str,
        hint: Option<&VersionRange>,
    ) -> Option<Rc<PackageVariantList>> {
        // Fast path: cached list whose underlying family map is still
        // the one the repo would return. Compare Rcs.
        if let Some(slot) = self.variant_lists.get(package_name) {
            match slot.as_ref() {
                Some(cached_list) => {
                    // Repo's get_family is cheap on a covered hint (no
                    // reload). If it returns the same Rc<FamilyMap> we
                    // already built the list against, the list is still
                    // valid.
                    let fresh_map = ctx.repo.get_family(package_name, hint);
                    if let Some(fresh_map) = fresh_map.as_ref() {
                        if Rc::ptr_eq(fresh_map, &cached_list.versions) {
                            return Some(Rc::clone(cached_list));
                        }
                        // The map changed under us (widen-reload). Fall
                        // through to rebuild against the fresh map.
                    } else {
                        // Repo lost the family between calls — treat as
                        // absent.
                        self.variant_lists.insert(Name::from(package_name), None);
                        return None;
                    }
                }
                None => {
                    // Previously absent. If the hint is wider than what
                    // produced the absence answer (or absence answer was
                    // produced with None hint), we'd see the change via
                    // the repo. But the repo encodes the previous hint
                    // and only retries when widening, so a fresh call
                    // here is the right gate.
                    let fresh = ctx.repo.get_family(package_name, hint);
                    if fresh.is_none() {
                        return None;
                    }
                    // Repo reloaded and now has the family — fall through
                    // to build a list from it.
                }
            }
        }
        // Slow path: build a fresh PackageVariantList from whatever the
        // repo currently has.
        let built = PackageVariantList::new(ctx, package_name, hint).map(Rc::new);
        self.variant_lists
            .insert(Name::from(package_name), built.clone());
        built
    }
}

#[cfg(test)]
mod tests {
    use super::super::context::{FamilyMap, PackageRepo};
    use super::*;
    use crate::PackageData;

    // --- build_rank_map (package-orderer plugin) ------------------------

    fn ord(strs: &[&str]) -> Vec<String> {
        strs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn rank_map_permutation() {
        // A clean reorder — ranks follow the orderer's output order.
        let m = build_rank_map(&["1", "2", "3"], ord(&["3", "1", "2"]));
        assert_eq!(m["3"], 0);
        assert_eq!(m["1"], 1);
        assert_eq!(m["2"], 2);
    }

    #[test]
    fn rank_map_omitted_versions_sink() {
        // "2" is the only named version; "1" and "3" sink to the bottom
        // in input order.
        let m = build_rank_map(&["1", "2", "3"], ord(&["2"]));
        assert_eq!(m["2"], 0);
        assert_eq!(m["1"], 1);
        assert_eq!(m["3"], 2);
    }

    #[test]
    fn rank_map_unknown_versions_ignored() {
        // "9" was never an input version — it's dropped, not ranked.
        let m = build_rank_map(&["1", "2"], ord(&["9", "1", "2"]));
        assert_eq!(m.len(), 2);
        assert_eq!(m["1"], 0);
        assert_eq!(m["2"], 1);
    }

    #[test]
    fn rank_map_duplicate_uses_first_occurrence() {
        let m = build_rank_map(&["1", "2"], ord(&["1", "1", "2"]));
        assert_eq!(m["1"], 0);
        assert_eq!(m["2"], 1);
    }

    #[test]
    fn rank_map_empty_output_keeps_input_order() {
        // An orderer that returns nothing → every version keeps input
        // order, all ranked, no panic.
        let m = build_rank_map(&["1", "2", "3"], ord(&[]));
        assert_eq!(m["1"], 0);
        assert_eq!(m["2"], 1);
        assert_eq!(m["3"], 2);
    }

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
        let map: std::collections::HashMap<String, FamilyMap> = entries
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
            .collect();
        PackageRepo::from_map(map)
    }

    fn ctx_with(repo: PackageRepo, requests: &[&str]) -> Rc<SolverContext> {
        let request_list =
            RequirementList::new(requests.iter().map(|s| Requirement::parse(s)).collect());
        Rc::new(SolverContext::new(Rc::new(repo), request_list))
    }

    #[test]
    fn test_build_variants_no_variants() {
        let r = repo(vec![("foo", vec![("1.0", pkg(&["bar-2"], &[]))])]);
        let ctx = ctx_with(r, &["foo"]);
        let list = PackageVariantList::new(&ctx, "foo", None).unwrap();
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
        let list = PackageVariantList::new(&ctx, "foo", None).unwrap();
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
        let list = PackageVariantList::new(&ctx, "foo", None).unwrap();
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
