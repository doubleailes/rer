use serde::Deserialize;
use std::collections::HashMap;
use rer_version::requirement::Requirements;
use std::fs;


#[derive(Deserialize)]
pub struct LocalPackages(HashMap<String, HashMap<String, Vec<String>>>);

impl LocalPackages {
    pub fn get_versions(&self, package_name: &str) -> Vec<String> {
        match self.0.get(package_name) {
            Some(versions) => versions.keys().cloned().collect(),
            None => Vec::new(),
        }
    }
    pub fn get_dependencies(&self, package_name: &str, version: &str) -> Requirements {
        match self.0.get(package_name) {
            Some(versions) => match versions.get(version) {
                Some(dependencies) => Requirements::from_str(dependencies.iter().map(|x| x.as_str()).collect()),
                None => Requirements::from_str(Vec::new()),
            },
            None => Requirements::from_str(Vec::new()),
        }
    }
    pub fn build_from_json_path(path: &str) -> Self {
        let data = fs::read_to_string(path).expect("Unable to read file");
        serde_json::from_str(&data).expect("Unable to parse json")
    }
}