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
    fn search_package(&self, package_name: &str) -> Vec<PathBuf> {
        let mut result: Vec<PathBuf> = Vec::new();
        for p in &self.paths {
            let mut path = p.clone();
            path.push(package_name);
            if path.exists() {
                result.push(path);
            }
        }
        result
    }
    fn search_and_merge_simple_versions(package_paths: Vec<PathBuf>) -> Vec<RerVersion> {
        let mut result: Vec<String> = Vec::new();
        for path in package_paths {
            fs::read_dir(path).unwrap().for_each(|entry| {
                let entry = entry.unwrap();
                let path = entry.path();
                let version = path.file_name().unwrap().to_str().unwrap().to_string();
                result.push(version);
            });
        }
        result.into_iter().map(|x| x.try_into().unwrap()).collect()
    }
    fn fetch_dependencies(&self, package_name: &str, version: &str) -> Option<Requirements> {
        let mut path = String::new();
        for x in self.paths.iter() {
            let mut p = x.clone();
            p.push(package_name);
            p.push(version);
            if p.exists() {
                path = p.to_str().unwrap().to_string();
                break;
            }
        }
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
        Some(
            r.into_iter()
                .map(|x| {
                    let (name, range) = x.get_pubgrub();
                    (name, range)
                })
                .collect(),
        )
    }
    fn init_dependencies(&self) -> Option<DependencyConstraints<String, RerVersion>> {
        let r: Requirements = self.init_request.clone();
        Some(
            r.into_iter()
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
        let package_paths = self.search_package(package);
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
