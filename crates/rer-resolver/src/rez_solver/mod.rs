//! A faithful Rust port of rez's package solver
//! (`rez/src/rez/solver.py` and `rez/src/rez/version/_requirement.py`).
//!
//! It reproduces rez's own phase-based backtracking algorithm so that resolves
//! match rez 1:1, including its weak (`~`) / conflict (`!`) requirement
//! semantics and variant selection order.
//!
//! The port is built bottom-up:
//! - [`requirement`] — `Requirement` / `RequirementList`
//! - [`context`] — the shared, read-only solve context
//! - [`variant`] — variant data structures (`PackageVariant`, slices, cache)
//! - [`scope`] — `_PackageScope`, one family's evolving state
//! - [`failure`] — solver status and failure reasons
//! - [`graph`] — cycle detection and dependency ordering
//! - [`phase`] — `_ResolvePhase`, the resolve algorithm
//! - [`solver`] — `Solver`, the phase-stack driver

pub mod context;
pub mod failure;
pub mod graph;
pub mod phase;
pub mod requirement;
pub mod scope;
pub mod solver;
pub mod variant;

pub use context::{make_shared_cache, PackageRepo, SharedVariantCache, SolverContext};
pub use failure::{DependencyConflict, FailureReason, SolverStatus};
pub use phase::ResolvePhase;
pub use requirement::{Requirement, RequirementList};
pub use scope::{PackageScope, ScopeError, ScopeIntersect, ScopeReduce};
pub use solver::Solver;
pub use variant::{
    PackageEntry, PackageVariant, PackageVariantCache, PackageVariantList, PackageVariantSlice,
    Reduction,
};
