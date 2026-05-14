//! Shared solve context — the read-only data that rez hangs off `self.solver`
//! and that the variant structures, scopes and resolve phases all reference.

use super::requirement::RequirementList;
use crate::PackageData;
use std::collections::HashMap;

/// The in-memory package repository: `family -> version -> PackageData`.
///
/// This replaces rez's on-disk `iter_packages(paths)` — the data is already
/// loaded, so the port does not need rez's lazy package-loading machinery.
pub type PackageRepo = HashMap<String, HashMap<String, PackageData>>;

/// Read-only context for a single solve.
///
/// Mirrors the subset of rez's `Solver` state that the variant structures,
/// scopes and phases read: the package repository and the top-level request
/// list. (rez also threads package paths, orderers, filters and counters
/// through `self.solver`; the port uses the always-default orderer, no
/// filters, and an in-memory repo.)
#[derive(Debug)]
pub struct SolverContext {
    /// The package repository.
    pub repo: PackageRepo,
    /// The merged, optimised top-level request.
    pub request_list: RequirementList,
}

impl SolverContext {
    /// Build a context from a repository and an already-merged request list.
    pub fn new(repo: PackageRepo, request_list: RequirementList) -> Self {
        SolverContext { repo, request_list }
    }
}
