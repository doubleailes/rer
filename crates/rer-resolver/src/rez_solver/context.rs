//! Shared solve context — the read-only data (plus the variant cache) that rez
//! hangs off `self.solver` and that the variant structures, scopes and resolve
//! phases all reference.

use super::requirement::RequirementList;
use super::variant::{PackageVariantCache, PackageVariantSlice};
use crate::PackageData;
use rer_version::VersionRange;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// One package family's `version -> PackageData` map.
pub type FamilyMap = HashMap<String, PackageData>;

/// Callback invoked on the first lookup for a family that is not already in
/// the repo, with an optional version-range hint indicating the *current*
/// solver constraint on that family. Returns `(version_string, PackageData)`
/// pairs.
///
/// **Hint semantics (issue #92):**
///
/// - `None` — the solver needs *every* version of the family (e.g. an
///   unbounded request, or a backtrack-widen has invalidated a narrower
///   prior load). The shim must return all versions.
/// - `Some(range)` — the solver only needs versions inside `range`. The
///   shim **may** filter to versions intersecting it (rez's
///   `iter_packages(range_=...)` does exactly that). The shim is **allowed
///   to return a superset** — pyrer re-validates against current
///   constraints, so extra versions are merely wasted parse time.
/// - The shim must **not** silently drop versions outside the hint without
///   pyrer asking — pyrer caches the loaded range and re-calls the loader
///   with a widened range if the solver backtracks and needs more.
///
/// An empty result means "no such family" *under the supplied hint*. The
/// repo memoises this answer paired with the hint that produced it; a
/// later wider hint will retry the loader.
///
/// `pyrer` builds one of these from a Python callable. See the load_family
/// callback in `pyrer.solve()` for the Python-side contract.
pub type FamilyLoader = Box<dyn Fn(&str, Option<&VersionRange>) -> Vec<(String, PackageData)>>;

/// A pluggable package orderer. Given a family name and the version
/// strings of every candidate, returns those strings reordered
/// **most-preferred-first** — the order the solver should try them in.
///
/// `rer` defaults to rez's `SortedOrder(descending=True)` (highest
/// version first) when no orderer is set. A host overrides that by
/// supplying one of these — e.g. to order by PEP 440 semantics rather
/// than rez's native alphanumeric-token comparison.
///
/// The contract is advisory and pyrer is defensive: a version present
/// in the input but missing from the output sinks to the bottom
/// (least preferred); a version in the output that wasn't in the input
/// is ignored. The orderer is a *preference* function — it never
/// changes whether a solve succeeds, only which solution is found
/// first.
///
/// `pyrer` builds one of these from a registered `PackageOrderer`
/// plugin. Wrapped in `Rc` so the `Solver` constructor's `build_ctx`
/// closure can clone it cheaply (mirrors how the variant cache is
/// shared).
pub type FamilyOrderer = dyn Fn(&str, &[&str]) -> Vec<String>;

/// Records which version-range was passed to the loader when a family was
/// last loaded. Used by [`PackageRepo`] to decide whether a cached family
/// map can serve a fresh request without re-calling the loader.
#[derive(Clone, Debug)]
enum LoadedRange {
    /// Loaded with `None` hint — every version is in the cached map.
    /// Always sufficient for any subsequent request.
    Unconstrained,
    /// Loaded with `Some(range)` — only versions inside this range are
    /// guaranteed to be in the cached map.
    Bounded(VersionRange),
}

impl LoadedRange {
    /// True if `hint` is fully covered by the cached load — i.e. every
    /// version the caller could care about is already in our map.
    fn covers(&self, hint: Option<&VersionRange>) -> bool {
        match (self, hint) {
            (LoadedRange::Unconstrained, _) => true,
            (LoadedRange::Bounded(_), None) => false,
            (LoadedRange::Bounded(loaded), Some(want)) => {
                // want ⊆ loaded  ⟺  loaded ∩ want == want
                loaded.intersection(want).as_ref() == Some(want)
            }
        }
    }

    /// The union of two load ranges, used when widening to satisfy a
    /// hint that wasn't covered by the previous load.
    fn widened_with(&self, hint: Option<&VersionRange>) -> LoadedRange {
        match (self, hint) {
            (LoadedRange::Unconstrained, _) | (_, None) => LoadedRange::Unconstrained,
            (LoadedRange::Bounded(loaded), Some(want)) => LoadedRange::Bounded(loaded.union(want)),
        }
    }
}

#[derive(Clone, Debug)]
struct FamilyEntry {
    loaded_range: LoadedRange,
    /// `Some(map)` if the family is known to exist; `None` if the loader
    /// returned empty under `loaded_range` (i.e. "no such family within
    /// this range, possibly absent entirely if `loaded_range` is
    /// `Unconstrained`").
    map: Option<Rc<FamilyMap>>,
}

/// The package repository — `family -> version -> PackageData`.
///
/// Replaces rez's on-disk `iter_packages(paths)` for callers that have the
/// data already loaded. With a [`FamilyLoader`] attached (see
/// [`Self::with_loader`]) it can also discover families lazily, the way rez's
/// own solver does.
///
/// Lookups are routed through [`Self::get_family`], which:
/// 1. Returns the cached `Rc<FamilyMap>` if the family has been loaded with
///    a range that covers the request.
/// 2. Otherwise calls the loader (if any) with the widened range, replaces
///    the cache entry, and returns the new map.
/// 3. Returns `None` if there's no loader and the family isn't cached, or
///    if the loader returns empty under the widened range.
///
/// Construction:
/// - [`Self::from_map`] / `impl From<HashMap<…>>` — eager, no loader.
/// - [`Self::with_loader`] — lazy; the loader is consulted on miss.
#[derive(Default)]
pub struct PackageRepo {
    families: RefCell<HashMap<String, FamilyEntry>>,
    loader: Option<FamilyLoader>,
}

impl std::fmt::Debug for PackageRepo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PackageRepo")
            .field("families", &self.families)
            .field("loader", &self.loader.as_ref().map(|_| "<callback>"))
            .finish()
    }
}

impl PackageRepo {
    /// Empty repo, no loader. Mostly useful as a starting point for tests
    /// that build the repo via [`Self::insert_family`].
    pub fn empty() -> Self {
        Self::default()
    }

    /// Eager repo from a `family -> version -> PackageData` map. The loader
    /// is `None`, so any family not in `map` is reported as absent on lookup.
    /// Eager-seeded families count as fully loaded (any range hint covered).
    pub fn from_map(map: HashMap<String, FamilyMap>) -> Self {
        let families = map
            .into_iter()
            .map(|(name, fam)| {
                (
                    name,
                    FamilyEntry {
                        loaded_range: LoadedRange::Unconstrained,
                        map: Some(Rc::new(fam)),
                    },
                )
            })
            .collect();
        PackageRepo {
            families: RefCell::new(families),
            loader: None,
        }
    }

    /// Repo backed by a loader. The loader is called the first time the
    /// solver asks for a family that isn't already cached — both hits and
    /// "no such family" answers are memoised. With the issue #92
    /// version-range hint, the loader may be re-called for the same family
    /// if a later request needs a wider range than the cached load
    /// covered; otherwise the cache hit is one-and-done.
    ///
    /// Use [`Self::insert_family`] to pre-seed families that are already
    /// in memory (e.g. ones produced by the caller's BFS seed pass).
    pub fn with_loader(loader: FamilyLoader) -> Self {
        PackageRepo {
            families: RefCell::default(),
            loader: Some(loader),
        }
    }

    /// Pre-populate a family. Counts as a full load (any range hint
    /// covered). Useful with [`Self::with_loader`] to skip the loader for
    /// families already in memory.
    pub fn insert_family(&self, name: String, fam: FamilyMap) {
        self.families.borrow_mut().insert(
            name,
            FamilyEntry {
                loaded_range: LoadedRange::Unconstrained,
                map: Some(Rc::new(fam)),
            },
        );
    }

    /// Number of present families currently cached in the repo. With a
    /// loader attached this grows as the solve progresses — it only
    /// reflects the eager-seeded set + whatever the loader has been
    /// asked for so far.
    pub fn family_count(&self) -> usize {
        self.families
            .borrow()
            .values()
            .filter(|entry| entry.map.is_some())
            .count()
    }

    /// `Some(family map)` if the family exists (cached or lazily loaded);
    /// `None` if there's no loader and it isn't cached, or if the loader
    /// returned no entries for it.
    ///
    /// The `hint` is the range the solver currently needs — passed through
    /// to the loader, which may use it to pre-filter (e.g. via rez's
    /// `iter_packages(range_=...)`). The repo tracks which range each
    /// family was loaded under and reloads (with a widened range) when a
    /// later request can't be served from the cache.
    pub fn get_family(&self, name: &str, hint: Option<&VersionRange>) -> Option<Rc<FamilyMap>> {
        // Cache hit + range covered → return directly.
        if let Some(entry) = self.families.borrow().get(name) {
            if entry.loaded_range.covers(hint) {
                return entry.map.clone();
            }
        }
        // Either uncached, or cached with a range that doesn't cover the
        // request. Reload with the widened range.
        let new_range = {
            let families = self.families.borrow();
            match families.get(name) {
                Some(entry) => entry.loaded_range.widened_with(hint),
                None => match hint {
                    None => LoadedRange::Unconstrained,
                    Some(r) => LoadedRange::Bounded(r.clone()),
                },
            }
        };

        let new_map = self.loader.as_ref().and_then(|load| {
            let widened_hint = match &new_range {
                LoadedRange::Unconstrained => None,
                LoadedRange::Bounded(r) => Some(r),
            };
            let entries = load(name, widened_hint);
            if entries.is_empty() {
                None
            } else {
                Some(Rc::new(entries.into_iter().collect::<HashMap<_, _>>()))
            }
        });

        self.families.borrow_mut().insert(
            name.to_string(),
            FamilyEntry {
                loaded_range: new_range,
                map: new_map.clone(),
            },
        );
        new_map
    }
}

impl From<HashMap<String, FamilyMap>> for PackageRepo {
    fn from(map: HashMap<String, FamilyMap>) -> Self {
        Self::from_map(map)
    }
}

/// A `PackageVariantCache` that can be shared between solves of the same
/// repository. Building a variant list (parsing every variant's requires)
/// accounts for roughly 22 % of solve time; sharing the cache across solves
/// of the same repo amortises that work over every request that touches a
/// given family.
///
/// The cache must only be shared between solves of the **same** repository —
/// it memoises "family present?" and the parsed `PackageVariantList`, both of
/// which would be wrong against a different repo.
pub type SharedVariantCache = Rc<RefCell<PackageVariantCache>>;

/// Build an empty cache that can be passed to `Solver::new_with_cache` (and
/// then reused across many solves of the same repository).
pub fn make_shared_cache() -> SharedVariantCache {
    Rc::new(RefCell::new(PackageVariantCache::new()))
}

/// How a [`PackageVariantSlice`] orders its variants — mirrors rez's
/// `config.variant_select_mode` (`solver.py:59-65`).
///
/// rez's `_PackageEntry.sort` is shared between the two modes:
/// `version_priority` orders by *which versions* of the requested packages
/// each variant pulls in, while `intersection_priority` orders first by
/// *how many* of the requested packages each variant satisfies (and only
/// then falls back to the version_priority key for ties).
///
/// `VersionPriority` is rez's default and is what the differential test
/// validates against. `IntersectionPriority` is opt-in for callers whose
/// rezconfig has it set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VariantSelectMode {
    /// rez's default. Pick the variant using the highest versions of the
    /// packages already in the request.
    #[default]
    VersionPriority,
    /// Pick the variant matching the *most* of the request, with the
    /// version_priority key as a tiebreak (`solver.py:449`).
    IntersectionPriority,
}

/// Context for a single solve.
///
/// Mirrors the subset of rez's `Solver` state that the variant structures,
/// scopes and phases read: the package repository, the top-level request list,
/// and the variant cache. The cache is wrapped in `Rc<RefCell<…>>` so that it
/// can be shared between solvers of the same repository — see
/// [`SharedVariantCache`].
pub struct SolverContext {
    /// The package repository, shared (never cloned per solve).
    pub repo: Rc<PackageRepo>,
    /// The merged, optimised top-level request.
    pub request_list: RequirementList,
    /// rez's `config.variant_select_mode` — selects the variant ordering
    /// key. Defaults to `VersionPriority` to match rez out of the box.
    pub variant_select_mode: VariantSelectMode,
    /// Optional pluggable package orderer (see [`FamilyOrderer`]). `None`
    /// means the default version-descending order. When set, it decides
    /// the per-family version preference the solver explores.
    pub package_order: Option<Rc<FamilyOrderer>>,
    /// Per-family variant cache. Sharing it across solves of the same repo is
    /// the dominant single optimisation: it skips re-parsing every variant's
    /// requires every solve.
    cache: SharedVariantCache,
}

// Manual `Debug` — `Rc<FamilyOrderer>` (an `Rc<dyn Fn>`) is not `Debug`.
impl std::fmt::Debug for SolverContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SolverContext")
            .field("repo", &self.repo)
            .field("request_list", &self.request_list)
            .field("variant_select_mode", &self.variant_select_mode)
            .field(
                "package_order",
                &self.package_order.as_ref().map(|_| "<set>"),
            )
            .field("cache", &self.cache)
            .finish()
    }
}

impl SolverContext {
    /// Build a context from a repository and an already-merged request list.
    /// A fresh, unshared variant cache is created — every solve will rebuild
    /// it from scratch. For repeated solves against the same repo, use
    /// [`Self::new_with_cache`]. Uses the default variant-select mode
    /// (`version_priority`).
    pub fn new(repo: Rc<PackageRepo>, request_list: RequirementList) -> Self {
        SolverContext {
            repo,
            request_list,
            variant_select_mode: VariantSelectMode::default(),
            package_order: None,
            cache: make_shared_cache(),
        }
    }

    /// Build a context with a pre-existing variant cache. The caller is
    /// responsible for ensuring the cache was built from the same repository.
    /// Uses the default variant-select mode (`version_priority`).
    pub fn new_with_cache(
        repo: Rc<PackageRepo>,
        request_list: RequirementList,
        cache: SharedVariantCache,
    ) -> Self {
        SolverContext {
            repo,
            request_list,
            variant_select_mode: VariantSelectMode::default(),
            package_order: None,
            cache,
        }
    }

    /// Replace this context's variant-select mode. Chainable on the
    /// builder-style constructors. Note: when sharing a cache across
    /// solves with different modes, take care — the cached
    /// `PackageEntry.sorted` flag depends on the mode used to compute it.
    pub fn with_variant_select_mode(mut self, mode: VariantSelectMode) -> Self {
        self.variant_select_mode = mode;
        self
    }

    /// Set this context's pluggable package orderer (see [`FamilyOrderer`]).
    /// `None` keeps the default version-descending order. Chainable on the
    /// builder-style constructors.
    ///
    /// Caveat — same shape as [`Self::with_variant_select_mode`]: a
    /// [`SharedVariantCache`] reused across solves must use a **consistent**
    /// orderer. A cached `PackageVariantList` bakes in orderer-specific
    /// `order_rank` values; reusing it under a different orderer would
    /// silently apply the stale ranking. `pyrer` is unaffected — it builds a
    /// fresh cache per `solve()`.
    pub fn with_package_order(mut self, orderer: Option<Rc<FamilyOrderer>>) -> Self {
        self.package_order = orderer;
        self
    }

    /// A slice of `name`'s variants intersected with `range`, or `None` if the
    /// family is absent or no version falls in range. Mirrors rez's
    /// `Solver._get_variant_slice`.
    pub fn get_variant_slice(
        self: &Rc<Self>,
        name: &str,
        range: &VersionRange,
    ) -> Option<PackageVariantSlice> {
        self.cache.borrow_mut().get_variant_slice(self, name, range)
    }

    /// True if `name` is not a family in the repository at all (as opposed to
    /// merely having no version in some range).
    pub fn family_missing(self: &Rc<Self>, name: &str) -> bool {
        self.cache.borrow_mut().family_missing(self, name)
    }
}
