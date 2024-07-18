use pubgrub::range::Range;
use pubgrub::solver::choose_package_with_fewest_versions;
use pubgrub::solver::Dependencies;
use pubgrub::solver::{DependencyConstraints, DependencyProvider};
use rer_version::requirement::Requirements;
use rer_version::RerVersion;
use std::borrow::Borrow;
use std::collections::HashMap;
use std::error::Error;
use std::fmt::Debug;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use test_rust_python::Package;

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct RerDependencyProvider {
    paths: Vec<PathBuf>,
    data: Arc<Mutex<HashMap<String, HashMap<RerVersion, String>>>>,
    init_request: Requirements,
    conflicted: Arc<Mutex<HashMap<String,Requirements>>>,
}

impl RerDependencyProvider {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn add_init_request(&mut self, init_request: Vec<String>) {
        let reduced =
            Requirements::from_str(init_request.iter().map(|x| x.as_str()).collect()).merge();
        self.init_request = reduced;
    }
    fn search_and_merge_simple_versions(package_paths: Vec<PathBuf>) -> Vec<RerVersion> {
        package_paths.into_iter()
            .flat_map(|path| fs::read_dir(path).unwrap_or_else(|_| panic!("Failed to read directory")))
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter_map(|path| path.file_name().and_then(|name| name.to_str()).map(|s| s.to_string()))
            .map(|version| version.try_into().unwrap_or_else(|_| panic!("Failed to convert version")))
            .collect()
    }
    fn fetch_dependencies(&self, package_name: &str, version: &str) -> Option<Requirements> {
        let path = self.paths.iter()
        .map(|base_path| base_path.join(package_name).join(version))
        .find(|p| p.exists())
        .and_then(|p| p.to_str().map(String::from))
        .unwrap_or_default();
        let package = Package::from_file(&format!("{}/package.py", path)).ok()?;
        let r = Requirements::from_str(
            package
                .get_dependencies()
                .iter()
                .map(|x| x.as_str())
                .collect(),
        )
        .merge();
        Some(r)
    }
    fn dependencies(
        &self,
        package: &String,
        version: &RerVersion,
    ) -> Option<DependencyConstraints<String, RerVersion>> {
        let r = self.fetch_dependencies(package, version.to_string().as_str())?;
        let (no_conflict, _conflict) = r.split_conflict();
        Some(
            no_conflict.into_iter()
                .map(|x| {
                    let (name, range) = x.get_pubgrub();
                    (name, range)
                })
                .collect(),
        )
    }
    fn init_dependencies(&self) -> Option<DependencyConstraints<String, RerVersion>> {
        let r: Requirements = self.init_request.clone();
        let (no_conflict, _conflict) = r.split_conflict();
        println!("Extracted request {}", no_conflict);
        Some(
            no_conflict.into_iter()
                .map(|x| {
                    let (name, range) = x.get_pubgrub();
                    (name, range)
                })
                .collect(),
        )
    }
    fn get_potential_versions(&self, package: &String) -> impl Iterator<Item = RerVersion> {
        if package == "init" {
            let unique_versions: Vec<RerVersion> = vec![RerVersion::from_str("1.0.0").unwrap()];
            return unique_versions.into_iter();
        }
        let package_paths: Vec<PathBuf> = self.paths
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
    pub fn lazy_paths(paths: Vec<PathBuf>) -> Self {
        Self {
            paths,
            ..Default::default()
        }
    }
}

impl DependencyProvider<String, RerVersion> for RerDependencyProvider {
    fn choose_package_version<T: Borrow<String>, U: Borrow<Range<RerVersion>>>(
        &self,
        potential_packages: impl Iterator<Item = (T, U)>,
    ) -> Result<(T, Option<RerVersion>), Box<dyn Error>> {
        Ok(choose_package_with_fewest_versions(
            |p| self.get_potential_versions(p),
            potential_packages,
        ))
    }
    fn get_dependencies(
        &self,
        package: &String,
        version: &RerVersion,
    ) -> Result<Dependencies<String, RerVersion>, Box<dyn Error>> {
        if package == "init" {
            return Ok(match self.init_dependencies() {
                None => Dependencies::Unknown,
                Some(dependencies) => Dependencies::Known(dependencies),
            });
        }
        Ok(match self.dependencies(package, version) {
            None => Dependencies::Unknown,
            Some(dependencies) => Dependencies::Known(dependencies),
        })
    }
}
