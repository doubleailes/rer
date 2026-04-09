use rer_version::Requirements;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

pub struct LocalPackages {
    data: HashMap<String, HashMap<String, String>>,
    paths: Vec<PathBuf>,
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
        match self.data.get(package_name) {
            Some(versions) => match versions.get(version) {
                Some(_path) => {
                    // Legacy Python package.py parsing has been removed
                    Requirements::empty()
                }
                None => panic!("Path not found"),
            },
            None => panic!("Package not found"),
        }
    }
    pub fn build_from_json_path(path: &str) -> Self {
        let data_str = fs::read_to_string(path).expect("Unable to read file");
        let data: HashMap<String, HashMap<String, String>> =
            serde_json::from_str(&data_str).expect("Unable to parse json");
        LocalPackages {
            data,
            paths: Vec::new(),
        }
    }
    pub fn lazy_paths(paths: Vec<PathBuf>) -> Self {
        LocalPackages {
            data: HashMap::new(),
            paths,
        }
    }
}

#[test]
fn test_search_package() {
    let path = "/home/philippe.llerena/workspace/github.com/doubleailes/rer-bkp/data_set/packages";
    let expected = path.to_string() + "/many";
    let local_packages = LocalPackages::lazy_paths(vec![PathBuf::from(path)]);
    let result = local_packages.search_package("many");
    assert_eq!(result, vec![PathBuf::from(expected)]);
}

#[test]
fn test_search_and_merge_versions() {
    let path = "/home/philippe.llerena/workspace/github.com/doubleailes/rer-bkp/data_set/packages";
    let local_packages = LocalPackages::lazy_paths(vec![PathBuf::from(path)]);
    let result =
        local_packages.search_and_merge_versions(vec![PathBuf::from(path.to_string() + "/many")]);
    assert_eq!(result, {
        let mut map = HashMap::new();
        map.insert("1.2.0".to_string(), path.to_string() + "/many/1.2.0");
        map
    });
}
