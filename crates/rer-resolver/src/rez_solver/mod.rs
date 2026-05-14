//! A faithful Rust port of rez's package solver
//! (`rez/src/rez/solver.py` and `rez/src/rez/version/_requirement.py`).
//!
//! Unlike the pubgrub-based solver in [`crate::solver`], this module reproduces
//! rez's own phase-based backtracking algorithm so that resolves match rez
//! 1:1, including its weak (`~`) / conflict (`!`) requirement semantics and
//! variant selection order.
//!
//! The port is built bottom-up:
//! - [`requirement`] — `Requirement` / `RequirementList`
//! - [`context`] — the shared, read-only solve context
//! - [`variant`] — variant data structures (`PackageVariant`, slices, cache)
//! - (further phases: scopes, resolve phases, solver)

pub mod context;
pub mod requirement;
pub mod variant;

pub use context::{PackageRepo, SolverContext};
pub use requirement::{Requirement, RequirementList};
pub use variant::{
    PackageEntry, PackageVariant, PackageVariantCache, PackageVariantList, PackageVariantSlice,
    Reduction,
};
