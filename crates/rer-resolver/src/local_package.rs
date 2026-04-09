use rer_version::Requirements;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// Data for a single package version, including base requirements and variant-specific dependencies.
///
/// For a package like:
/// ```text
/// name = "maya_utils"
/// version = "1.0.0"
/// requires = ["python-3"]
/// variants = [["maya-2024"], ["maya-2025"]]
/// ```
///
/// `requires` would be `["python-3"]` and `variants` would be `[["maya-2024"], ["maya-2025"]]`.
#[derive(Clone, Debug, Default)]
pub struct PackageData {
    /// Base requirements that apply to all variants.
    pub requires: Vec<String>,
    /// Each inner Vec is a list of dependency strings for one variant.
    /// Empty if the package has no variants.
    pub variants: Vec<Vec<String>>,
}

impl PackageData {
    /// Returns true if the package has more than one variant.
    pub fn is_multi_variant(&self) -> bool {
        self.variants.len() > 1
    }

    /// Returns the combined requirements for a single-variant or no-variant package.
    /// For multi-variant packages, returns only the base requires.
    pub fn combined_requirements(&self) -> Vec<String> {
        let mut deps = self.requires.clone();
        if self.variants.len() == 1 {
            deps.extend(self.variants[0].clone());
        }
        deps
    }
}

pub struct LocalPackages {
    data: HashMap<String, HashMap<String, String>>,
    paths: Vec<PathBuf>,
    /// Variant-aware package data: package_name → version → PackageData
    package_data: HashMap<String, HashMap<String, PackageData>>,
}

impl LocalPackages {
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
    pub fn get_versions(&mut self, package_name: &str) -> Vec<String> {
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
    pub fn get_dependencies(&self, package_name: &str, version: &str) -> Requirements {
        // First check variant-aware package data
        if let Some(pkg_data) = self.get_package_data(package_name, version) {
            let deps = pkg_data.combined_requirements();
            if !deps.is_empty() {
                return Requirements::from(deps.iter().map(|s| s.as_str()).collect());
            }
        }
        match self.data.get(package_name) {
            Some(versions) => match versions.get(version) {
                Some(_path) => {
                    // Legacy Python package.py parsing has been removed
                    Requirements::empty()
                }
                None => panic!("Path not found"),
            },
            None => {
                // If not in filesystem data, check package_data
                if self.package_data.contains_key(package_name) {
                    return Requirements::empty();
                }
                panic!("Package not found")
            }
        }
    }
    /// Get variant-aware package data for a specific package version.
    pub fn get_package_data(&self, package_name: &str, version: &str) -> Option<PackageData> {
        self.package_data
            .get(package_name)
            .and_then(|versions| versions.get(version))
            .cloned()
    }
    pub fn build_from_json_path(path: &str) -> Self {
        let data_str = fs::read_to_string(path).expect("Unable to read file");
        let data: HashMap<String, HashMap<String, String>> =
            serde_json::from_str(&data_str).expect("Unable to parse json");
        LocalPackages {
            data,
            paths: Vec::new(),
            package_data: HashMap::new(),
        }
    }
    pub fn lazy_paths(paths: Vec<PathBuf>) -> Self {
        LocalPackages {
            data: HashMap::new(),
            paths,
            package_data: HashMap::new(),
        }
    }
    /// Construct a `LocalPackages` from in-memory variant-aware package data.
    ///
    /// The input maps package names to their versions, where each version has
    /// a `PackageData` containing base requirements and optional variants.
    pub fn from_packages(
        packages: HashMap<String, HashMap<String, PackageData>>,
    ) -> Self {
        // Also populate the `data` field with version→"" mapping for get_versions()
        let mut data: HashMap<String, HashMap<String, String>> = HashMap::new();
        for (pkg_name, versions) in &packages {
            let version_map: HashMap<String, String> = versions
                .keys()
                .map(|v| (v.clone(), String::new()))
                .collect();
            data.insert(pkg_name.clone(), version_map);
        }
        LocalPackages {
            data,
            paths: Vec::new(),
            package_data: packages,
        }
    }
}

#[test]
fn test_search_package() {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/packages");
    let expected = base.join("many");
    let local_packages = LocalPackages::lazy_paths(vec![base]);
    let result = local_packages.search_package("many");
    assert_eq!(result, vec![expected]);
}

#[test]
fn test_search_and_merge_versions() {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/packages");
    let many_path = base.join("many");
    let local_packages = LocalPackages::lazy_paths(vec![base]);
    let result = local_packages.search_and_merge_versions(vec![many_path.clone()]);
    assert_eq!(result, {
        let mut map = HashMap::new();
        map.insert(
            "1.2.0".to_string(),
            many_path.join("1.2.0").to_str().unwrap().to_string(),
        );
        map
    });
}
