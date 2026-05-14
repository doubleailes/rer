//! Solver status and failure-reason types, ported from `rez/src/rez/solver.py`
//! (`SolverStatus`, `Reduction` lives in [`super::variant`], `DependencyConflict`,
//! `TotalReduction`, `DependencyConflicts`, `Cycle`).

use super::requirement::Requirement;
use super::variant::Reduction;
use rer_version::RerVersion;

/// The state of a resolve phase, or of the solver as a whole. Mirrors rez's
/// `SolverStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolverStatus {
    /// The phase has not yet been solved.
    Pending,
    /// The phase (or solve) is fully resolved.
    Solved,
    /// The phase reduced as far as it can without selecting a sub-range — it
    /// must be split.
    Exhausted,
    /// The phase (or solve) failed.
    Failed,
    /// The phase resolved but contains a dependency cycle.
    Cyclic,
    /// The solve is still in progress.
    Unsolved,
}

/// Two requirements that cannot both be satisfied. Mirrors rez's
/// `DependencyConflict`.
#[derive(Debug, Clone)]
pub struct DependencyConflict {
    /// The dependency requirement.
    pub dependency: Requirement,
    /// The request it conflicts with.
    pub conflicting_request: Requirement,
}

impl std::fmt::Display for DependencyConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} <--!--> {}",
            self.dependency, self.conflicting_request
        )
    }
}

/// Why a resolve phase failed. Mirrors rez's `FailureReason` hierarchy.
#[derive(Debug, Clone)]
pub enum FailureReason {
    /// A package scope was reduced to nothing.
    TotalReduction(Vec<Reduction>),
    /// One or more requirement conflicts.
    DependencyConflicts(Vec<DependencyConflict>),
    /// A dependency cycle among the resolved packages.
    Cycle(Vec<(String, RerVersion)>),
}

impl FailureReason {
    /// A human-readable description of the failure.
    pub fn description(&self) -> String {
        match self {
            FailureReason::TotalReduction(reductions) => {
                let parts: Vec<String> = reductions.iter().map(|r| r.to_string()).collect();
                format!(
                    "the following package conflicts caused a complete reduction:\n{}",
                    parts.join("\n")
                )
            }
            FailureReason::DependencyConflicts(conflicts) => {
                let parts: Vec<String> = conflicts.iter().map(|c| c.to_string()).collect();
                format!(
                    "the following package conflicts occurred:\n{}",
                    parts.join("\n")
                )
            }
            FailureReason::Cycle(packages) => {
                let parts: Vec<String> = packages.iter().map(|(n, v)| format!("{n}-{v}")).collect();
                format!("a cyclic dependency was detected:\n{}", parts.join(" --> "))
            }
        }
    }
}
