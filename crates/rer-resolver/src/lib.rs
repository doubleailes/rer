mod candidate_selector;
pub use candidate_selector::{CandidateList, ResolutionMode};
mod local_package;
pub use local_package::{LocalPackages, PackageData};
mod solver;
pub use solver::{solver, solver_with_packages};
pub mod resolver;
