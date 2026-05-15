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

/// The in-memory package repository: `family -> version -> PackageData`.
///
/// This replaces rez's on-disk `iter_packages(paths)` — the data is already
/// loaded, so the port does not need rez's lazy package-loading machinery.
pub type PackageRepo = HashMap<String, HashMap<String, PackageData>>;

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
    /// Per-family variant cache. Sharing it across solves of the same repo is
    /// the dominant single optimisation: it skips re-parsing every variant's
    /// requires every solve.
    cache: SharedVariantCache,
}

impl SolverContext {
    /// Build a context from a repository and an already-merged request list.
    /// A fresh, unshared variant cache is created — every solve will rebuild
    /// it from scratch. For repeated solves against the same repo, use
    /// [`Self::new_with_cache`].
    pub fn new(repo: Rc<PackageRepo>, request_list: RequirementList) -> Self {
        SolverContext {
            repo,
            request_list,
            cache: make_shared_cache(),
        }
    }

    /// Build a context with a pre-existing variant cache. The caller is
    /// responsible for ensuring the cache was built from the same repository.
    pub fn new_with_cache(
        repo: Rc<PackageRepo>,
        request_list: RequirementList,
        cache: SharedVariantCache,
    ) -> Self {
        SolverContext {
            repo,
            request_list,
            cache,
        }
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
