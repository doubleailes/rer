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
/// the repo. Returns `(version_string, PackageData)` pairs — every version of
/// the family. An empty result means "no such family"; the repo caches that
/// answer and never calls the loader for the same name again.
///
/// Mirrors the lazy-load behaviour rez gets from its `Package` resource
/// wrapper (each `package.py` is AST-evaluated on first attribute access).
/// `pyrer` builds one of these from a Python callable for issue #86.
pub type FamilyLoader = Box<dyn Fn(&str) -> Vec<(String, PackageData)>>;

/// The package repository — `family -> version -> PackageData`.
///
/// Replaces rez's on-disk `iter_packages(paths)` for callers that have the
/// data already loaded. With a [`FamilyLoader`] attached (see
/// [`Self::with_loader`]) it can also discover families lazily, the way rez's
/// own solver does.
///
/// Lookups are routed through [`Self::get_family`], which:
/// 1. Returns the cached `Rc<FamilyMap>` if the family has been seen.
/// 2. Otherwise calls the loader (if any), memoising both the hit and the
///    "no such family" answer.
/// 3. Otherwise returns `None`.
///
/// Construction:
/// - [`Self::from_map`] / `impl From<HashMap<…>>` — eager, no loader.
/// - [`Self::with_loader`] — lazy; the loader is consulted on miss.
#[derive(Default)]
pub struct PackageRepo {
    /// `Some(map)` for present families, `None` for families the loader
    /// confirmed as absent (so we don't re-call it on miss).
    families: RefCell<HashMap<String, Option<Rc<FamilyMap>>>>,
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
    pub fn from_map(map: HashMap<String, FamilyMap>) -> Self {
        let families = map
            .into_iter()
            .map(|(name, fam)| (name, Some(Rc::new(fam))))
            .collect();
        PackageRepo {
            families: RefCell::new(families),
            loader: None,
        }
    }

    /// Repo backed by a loader. The loader is called the first time the
    /// solver asks for a family that isn't already cached — both hits and
    /// "no such family" answers are memoised, so the loader fires at most
    /// once per family per repo.
    ///
    /// Use [`Self::insert_family`] to pre-seed families that are already
    /// in memory (e.g. ones produced by the caller's BFS seed pass).
    pub fn with_loader(loader: FamilyLoader) -> Self {
        PackageRepo {
            families: RefCell::default(),
            loader: Some(loader),
        }
    }

    /// Pre-populate a family. Useful with [`Self::with_loader`] to skip
    /// the loader for families already in memory.
    pub fn insert_family(&self, name: String, fam: FamilyMap) {
        self.families.borrow_mut().insert(name, Some(Rc::new(fam)));
    }

    /// Number of families currently cached in the repo. With a loader
    /// attached this grows as the solve progresses — it only reflects the
    /// eager-seeded set + whatever the loader has been asked for so far.
    pub fn family_count(&self) -> usize {
        self.families
            .borrow()
            .values()
            .filter(|v| v.is_some())
            .count()
    }

    /// `Some(family map)` if the family exists (cached or lazily loaded);
    /// `None` if there's no loader and it isn't cached, or if the loader
    /// returned no entries for it.
    pub fn get_family(&self, name: &str) -> Option<Rc<FamilyMap>> {
        if let Some(slot) = self.families.borrow().get(name) {
            return slot.clone();
        }
        let loaded = self.loader.as_ref().and_then(|load| {
            let entries = load(name);
            if entries.is_empty() {
                None
            } else {
                Some(Rc::new(entries.into_iter().collect::<HashMap<_, _>>()))
            }
        });
        self.families
            .borrow_mut()
            .insert(name.to_string(), loaded.clone());
        loaded
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
#[derive(Debug)]
pub struct SolverContext {
    /// The package repository, shared (never cloned per solve).
    pub repo: Rc<PackageRepo>,
    /// The merged, optimised top-level request.
    pub request_list: RequirementList,
    /// rez's `config.variant_select_mode` — selects the variant ordering
    /// key. Defaults to `VersionPriority` to match rez out of the box.
    pub variant_select_mode: VariantSelectMode,
    /// Per-family variant cache. Sharing it across solves of the same repo is
    /// the dominant single optimisation: it skips re-parsing every variant's
    /// requires every solve.
    cache: SharedVariantCache,
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
