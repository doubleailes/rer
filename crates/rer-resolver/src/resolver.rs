use pubgrub::range::Range;
use pubgrub::solver::{DependencyConstraints, DependencyProvider};
use pubgrub::type_aliases::Map;
use rer_version::RerVersion;
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::path::PathBuf;
use pubgrub::solver::Dependencies;
use std::error::Error;
use rer_version::requirement::{Requirement,Requirements};
use std::collections::HashMap;
use test_rust_python::Package;
use std::fs;
use std::borrow::Borrow;
use pubgrub::solver::choose_package_with_fewest_versions;

#[derive(Debug, Clone, Default)]
pub struct RerDependencyProvider {
    dependencies: Map<String, BTreeMap<RerVersion, DependencyConstraints<String, RerVersion>>>,
    // For examplae python is constained by Maya at a certaun to a specific version ( of python )
    weak_refeence: Map<String, Map<String, DependencyConstraints<RerVersion, RerVersion>>>,
    data:HashMap<String, HashMap<String, String>>,
    paths: Vec<PathBuf>
}

impl RerDependencyProvider {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn add_dependencies(
        &mut self,
        package: String,
        version: RerVersion,
        dependencies: Requirements,
    ) {
        let package_deps = dependencies.into_iter().map(|x| {
            let (name, range) = x.get_pubgrub();
            (name, range)
        }).collect();
        *self
            .dependencies
            .entry(package)
            .or_default()
            .entry(version)
            .or_default() = package_deps;
    }
    pub fn packages(&self) -> impl Iterator<Item = &String>{
        self.dependencies.keys()
    
    }
    pub fn versions(&self, package: &String) -> Option<impl Iterator<Item = &RerVersion>> {
        self.dependencies.get(package).map(|k| k.keys())
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
    fn search_and_merge_versions(&self, package_paths: Vec<PathBuf>) -> HashMap<String, String> {
        let mut result: HashMap<String, String> = HashMap::new();
        for path in package_paths {
            fs::read_dir(path).unwrap().for_each(|entry| {
                let entry = entry.unwrap();
                let path = entry.path();
                let version = path.file_name().unwrap().to_str().unwrap().to_string();
                result.insert(version, path.to_str().unwrap().to_string());
            });
        }
        result
    }
    fn get_str_versions(&mut self, package_name: &str) -> Vec<String> {
        if !self.data.contains_key(package_name) {
            let package_paths = self.search_package(package_name);
            let versions = self.search_and_merge_versions(package_paths);
            self.data.insert(package_name.to_string(), versions);
        }
        match self.data.get(package_name) {
            Some(versions) => versions.keys().cloned().collect(),
            None => Vec::new(),
        }
    }
    fn get_versions(&mut self, package_name: &str) -> Vec<RerVersion> {
        self.get_str_versions(package_name)
            .iter()
            .map(|x| x.as_str().try_into().unwrap())
            .collect()
    }
    fn fetch_dependencies(&self, package_name: &str, version: &str) -> Requirements {
        match self.data.get(package_name) {
            Some(versions) => match versions.get(version) {
                Some(path) => match Package::from_file(&format!("{}/package.py", path)) {
                    Ok(package) => Requirements::from_str(
                        package
                        .get_dependencies()
                        .iter()
                        .map(|x| x.as_str())
                        .collect(),
                    ),
                    Err(err) => panic!("Error reading package {}", err),
                },
                None => panic!("Path not found"),
            },
            None => panic!("Package not found"),
        }
    }
    fn dependencies(&self, package: &String, version: &RerVersion) -> Option<DependencyConstraints<String, RerVersion>> {
        let r = self.fetch_dependencies(package, version.to_string().as_str());
        if r.is_empty() {
            None
        } else {
            Some(r.into_iter().map(|x| {
                let (name, range) = x.get_pubgrub();
                (name, range)
            }).collect())
        }
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
            |p| {
                self.dependencies
                    .get(p)
                    .into_iter()
                    .flat_map(|k| k.keys())
                    .rev()
                    .cloned()
            },
            potential_packages,
        ))
    }
    fn get_dependencies(
        &self,
        package: &String,
        version: &RerVersion,
    ) -> Result<Dependencies<String, RerVersion>, Box<dyn Error>> {
        Ok(match self.dependencies(package, version) {
            None => Dependencies::Unknown,
            Some(dependencies) => Dependencies::Known(dependencies),
        })
    }
}

#[test]
fn test_dependency_provider() {
    let mut provider = RerDependencyProvider::new();
    let requirements_str = vec!["golaem==1.0", "nuke-15.4"];
    let req = Requirements::from_str(requirements_str);
    provider.add_dependencies("maya".to_string(), "1.0".try_into().unwrap() , req);
    let p: Vec<String> = provider.packages().map(|x| x.clone()).collect();
    assert_eq!(p, vec!["maya"]);
    let v: Vec<RerVersion> = provider.versions(&"maya".to_string()).unwrap().map(|x| x.clone()).collect();
    let expected: Vec<RerVersion> = vec!["1.0".try_into().unwrap()];
    assert_eq!(v, expected);
}