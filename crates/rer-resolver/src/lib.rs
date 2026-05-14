//! `rer-resolver` — rer's package solver.
//!
//! [`rez_solver`] is a faithful Rust port of rez's own phase-based backtracking
//! solver; resolves match rez 1:1, including weak (`~`) / conflict (`!`)
//! requirement semantics and variant selection order. [`PackageData`] is the
//! in-memory unit of the package repository it works on.

mod package_data;
pub use package_data::PackageData;

pub mod rez_solver;
