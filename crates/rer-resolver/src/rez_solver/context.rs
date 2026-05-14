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

/// Context for a single solve.
///
/// Mirrors the subset of rez's `Solver` state that the variant structures,
/// scopes and phases read: the package repository, the top-level request list,
/// and the variant cache (behind a `RefCell`, since building variant slices is
/// a logically read-only operation that mutates a memo).
#[derive(Debug)]
pub struct SolverContext {
    /// The package repository.
    pub repo: PackageRepo,
    /// The merged, optimised top-level request.
    pub request_list: RequirementList,
    /// Per-family variant cache — built lazily, shared for the whole solve.
    cache: RefCell<PackageVariantCache>,
}

impl SolverContext {
    /// Build a context from a repository and an already-merged request list.
    pub fn new(repo: PackageRepo, request_list: RequirementList) -> Self {
        SolverContext {
            repo,
            request_list,
            cache: RefCell::new(PackageVariantCache::new()),
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
