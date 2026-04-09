use crate::candidate_selector::{CandidateList, ResolutionMode};
use pubgrub::{Dependencies, DependencyConstraints, DependencyProvider, PackageResolutionStatistics};
use rer_version::Requirements;
use rer_version::RerVersion;
use std::cmp::Reverse;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use version_ranges::Ranges;

#[derive(Debug, Clone, Default)]
pub struct RerDependencyProvider {
    paths: Vec<PathBuf>,
    init_request: Requirements,
    conflicted: Arc<Mutex<Requirements>>,
    counter: Arc<Mutex<u32>>,
}

impl RerDependencyProvider {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn add_init_request(&mut self, init_request: Vec<String>) {
        let reduced = Requirements::from(init_request.iter().map(|x| x.as_str()).collect());
        self.init_request = self.fetch_and_merge_conflict(reduced);
    }
    fn search_and_merge_simple_versions(package_paths: Vec<PathBuf>) -> Vec<RerVersion> {
        let p: Vec<RerVersion> = package_paths
            .into_iter()
            .flat_map(|path| {
                fs::read_dir(path).unwrap_or_else(|_| panic!("Failed to read directory"))
            })
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter_map(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(|s| s.to_string())
            })
            .map(|version| {
                version
                    .try_into()
                    .unwrap_or_else(|_| panic!("Failed to convert version"))
            })
            .collect();
        p.into_iter().rev().collect()
    }
    fn fetch_and_merge_conflict(&self, in_request: Requirements) -> Requirements {
        let (no_conflict, conflict) = in_request.split_conflict();
        self.conflicted.lock().unwrap().extend(&conflict);
        no_conflict
    }
    fn fetch_dependencies(&self, package_name: &str, version: &str) -> Option<Requirements> {
        let path = self
            .paths
            .iter()
            .map(|base_path| base_path.join(package_name).join(version))
            .find(|p| p.exists())
            .and_then(|p| p.to_str().map(String::from))
            .unwrap_or_default();
        // Legacy Python package.py parsing has been removed
        let _ = path;
        Some(Requirements::empty())
    }
    fn dependencies(
        &self,
        package: &str,
        version: &RerVersion,
    ) -> Option<DependencyConstraints<String, Ranges<RerVersion>>> {
        let r = self.fetch_dependencies(package, version.to_string().as_str())?;
        Some(
            r.into_iter()
                .map(|x| {
                    let (name, range) = x.get_pubgrub();
                    (name, range)
                })
                .collect(),
        )
    }
    fn init_dependencies(&self) -> Option<DependencyConstraints<String, Ranges<RerVersion>>> {
        let r: Requirements = self.init_request.clone();
        let (no_conflict, _conflict) = r.split_conflict();
        Some(
            no_conflict
                .into_iter()
                .map(|x| {
                    let (name, range) = x.get_pubgrub();
                    (name, range)
                })
                .collect(),
        )
    }
    fn get_potential_versions(&self, package: &String) -> impl Iterator<Item = RerVersion> {
        if package == "init" {
            let unique_versions: Vec<RerVersion> = vec![RerVersion::try_from("1.0.0").unwrap()];
            return unique_versions.into_iter();
        }
        let package_paths: Vec<PathBuf> = self
            .paths
            .iter()
            .map(|x| {
                let mut p = x.clone();
                p.push(package);
                p
            })
            .filter(|x| x.exists())
            .collect();
        Self::search_and_merge_simple_versions(package_paths).into_iter()
    }
    fn candidat_selector(
        &self,
        mut potential_packages: impl Iterator<Item = (String, Ranges<RerVersion>)>,
    ) -> (String, Option<RerVersion>) {
        let (pkg, range) = potential_packages.find(|_| true).unwrap();
        let package_paths: Vec<PathBuf> = self
            .paths
            .iter()
            .map(|x| {
                let mut p = x.clone();
                p.push(pkg.clone());
                p
            })
            .filter(|x| x.exists())
            .collect();
        let mut t = self.conflicted.lock().unwrap();
        let (range, requirements) = t.reduced(&pkg, &range);
        t.switch(&requirements);
        drop(t);
        let v = CandidateList::new(Self::search_and_merge_simple_versions(package_paths))
            .find_candidate(&range, ResolutionMode::Highest);
        (pkg, v)
    }
    pub fn lazy_paths(paths: Vec<PathBuf>) -> Self {
        Self {
            paths,
            ..Default::default()
        }
    }
}

/// Custom error type for the solver.
#[derive(Debug)]
pub struct RerSolverError(String);

impl fmt::Display for RerSolverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Error for RerSolverError {}

impl DependencyProvider for RerDependencyProvider {
    type P = String;
    type V = RerVersion;
    type VS = Ranges<RerVersion>;
    type M = String;
    type Priority = Reverse<usize>;
    type Err = RerSolverError;

    fn prioritize(
        &self,
        package: &Self::P,
        range: &Self::VS,
        _package_conflicts_counts: &PackageResolutionStatistics,
    ) -> Self::Priority {
        if package == "init" {
            return Reverse(0);
        }
        let package_paths: Vec<PathBuf> = self
            .paths
            .iter()
            .map(|x| {
                let mut p = x.clone();
                p.push(package);
                p
            })
            .filter(|x| x.exists())
            .collect();
        let versions = Self::search_and_merge_simple_versions(package_paths);
        let count = versions.iter().filter(|v| range.contains(v)).count();
        Reverse(count)
    }

    fn choose_version(
        &self,
        package: &Self::P,
        range: &Self::VS,
    ) -> Result<Option<Self::V>, Self::Err> {
        if package == "init" {
            let unique_version: RerVersion = RerVersion::try_from("1.0.0").unwrap();
            return Ok(Some(unique_version));
        }
        let package_paths: Vec<PathBuf> = self
            .paths
            .iter()
            .map(|x| {
                let mut p = x.clone();
                p.push(package);
                p
            })
            .filter(|x| x.exists())
            .collect();
        let mut t = self.conflicted.lock().unwrap();
        let (range, requirements) = t.reduced(package, range);
        t.switch(&requirements);
        drop(t);
        let v = CandidateList::new(Self::search_and_merge_simple_versions(package_paths))
            .find_candidate(&range, ResolutionMode::Highest);
        Ok(v)
    }

    fn get_dependencies(
        &self,
        package: &Self::P,
        version: &Self::V,
    ) -> Result<Dependencies<Self::P, Self::VS, Self::M>, Self::Err> {
        if package == "init" {
            return Ok(match self.init_dependencies() {
                None => Dependencies::Unavailable("dependencies unavailable".to_string()),
                Some(dependencies) => Dependencies::Available(dependencies),
            });
        }
        self.should_cancel()?;
        Ok(match self.dependencies(package, version) {
            None => Dependencies::Unavailable("dependencies unavailable".to_string()),
            Some(dependencies) => Dependencies::Available(dependencies),
        })
    }

    fn should_cancel(&self) -> Result<(), Self::Err> {
        let mut counter = self.counter.lock().unwrap();
        if *counter > 1000 {
            return Err(RerSolverError("Too many iterations".to_string()));
        }
        *counter += 1;
        drop(counter);
        Ok(())
    }
}
