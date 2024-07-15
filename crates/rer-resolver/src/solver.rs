use crate::candidate_selector::CandidateList;
use crate::LocalPackages;
use pubgrub::error::PubGrubError;
use pubgrub::range::Range;
use pubgrub::solver::DependencyProvider;
use pubgrub::solver::{resolve, OfflineDependencyProvider};
use rer_version::requirement::Requirement;
use rer_version::{requirement::Requirements, RerVersion};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

fn check_version<'a>(
    dependency_provider: &'a Arc<Mutex<OfflineDependencyProvider<String, RerVersion>>>,
    package_name: &'a String,
    version: &'a RerVersion,
) -> bool {
    let versions_package: Vec<RerVersion> =
        match dependency_provider.lock().unwrap().versions(&package_name) {
            Some(versions_package) => versions_package.into_iter().map(|x| x.clone()).collect(),
            None => Vec::new(),
        };
    if versions_package.contains(&&version) {
        return true;
    } else {
        false
    }
}

fn recursive(
    dependency_provider: &Arc<Mutex<OfflineDependencyProvider<String, RerVersion>>>,
    local_packages: &mut LocalPackages,
    dependencies: Requirements,
    cache_requierements: &Arc<Mutex<HashSet<Requirement>>>,
    weak_references: &Arc<Mutex<HashMap<String, Range<RerVersion>>>>,
) {
    for dependency in dependencies {
        // Acquire the mutex
        let mut cache_requi = cache_requierements.lock().unwrap();
        if cache_requi.contains(&dependency) {
            continue;
        } else {
            cache_requi.insert(dependency.clone());
        }
        // Drop the mutex to be sure it is not poison
        drop(cache_requi);
        let package_name = dependency.get_name().to_string();
        if dependency.is_weak_ref() {
            let range = dependency.get_version_range().unwrap_or(Range::any());
            let mut weak_references_guard = weak_references.lock().unwrap();
            if !weak_references_guard.contains_key(&package_name) {
                weak_references_guard.insert(package_name.clone(), range);
            } else {
                let current_range = weak_references_guard.get(&package_name).unwrap();
                let new_range = current_range.union(&range);
                weak_references_guard.insert(package_name.clone(), new_range);
            }
            drop(weak_references_guard);
        }
        let versions = local_packages.get_versions(&package_name);
        let candidates = CandidateList::from_vec_str(versions.iter().map(|x| x.as_str()).collect());
        let range = dependency.get_version_range().unwrap_or(Range::any());
        let candidates = candidates.find_candidates(&range);
        for candidate in candidates {
            if check_version(dependency_provider, &package_name, &candidate) {
                continue;
            }
            let dependencies =
                local_packages.get_dependencies(&package_name, &candidate.to_string());
            let dependencies_pubgrub: Vec<(String, Range<RerVersion>)> = dependencies.to_pubgrub();
            let mut dependency_provider_guard = dependency_provider.lock().unwrap();
            dependency_provider_guard.add_dependencies(
                package_name.to_string(),
                candidate.clone(),
                dependencies_pubgrub,
            );
            drop(dependency_provider_guard);
            if dependencies.is_empty() {
                continue;
            }
            recursive(
                dependency_provider,
                local_packages,
                dependencies,
                cache_requierements,
                weak_references,
            );
        }
    }
}

pub fn solver(
    requirements_str: Vec<&str>,
    paths: Vec<PathBuf>,
) -> Result<Vec<String>, PubGrubError<String, RerVersion>> {
    let dependency_provider: Arc<Mutex<OfflineDependencyProvider<String, RerVersion>>> = Arc::new(
        Mutex::new(OfflineDependencyProvider::<String, RerVersion>::new()),
    );
    let cache_requierements: Arc<Mutex<HashSet<Requirement>>> =
        Arc::new(Mutex::new(HashSet::new()));
    let weak_references: Arc<Mutex<HashMap<String, Range<RerVersion>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let mut packages = LocalPackages::lazy_paths(paths);
    // Create a uuid to represent the current request
    let context_name = Uuid::new_v4().to_string();
    // Transform the list of str into a list of Requirement and merge if possible
    let dependencies: Requirements = Requirements::from_str(requirements_str).merge();
    let mut new_deps = Requirements::empty();
    for dependency in dependencies {
        if dependency.is_weak_ref() {
            let name = dependency.get_name().to_string();
            let range = dependency.get_version_range().unwrap_or(Range::any());
            let mut weak_references_guard = weak_references.lock().unwrap();
            if !weak_references_guard.contains_key(&name) {
                weak_references_guard.insert(name, range);
            } else {
                let current_range = weak_references_guard.get(&name).unwrap();
                let new_range = current_range.union(&range);
                weak_references_guard.insert(name, new_range);
            }
            drop(weak_references_guard);
        } else {
            new_deps.add(dependency);
        }
    }
    let dependencies = new_deps;
    // Convert list of str into list of Requirement
    let dependencies_pubgrub: Vec<(String, Range<RerVersion>)> = dependencies.to_pubgrub();
    // clone the current version
    let v: RerVersion = "1.0.0".try_into().unwrap();
    // grab the mutex
    dependency_provider.lock().unwrap().add_dependencies(
        context_name.clone(),
        v.clone(),
        dependencies_pubgrub,
    );
    recursive(
        &dependency_provider,
        &mut packages,
        dependencies,
        &cache_requierements,
        &weak_references,
    );
    let p: OfflineDependencyProvider<String, RerVersion> =
        dependency_provider.lock().unwrap().clone();
    match resolve(&p, context_name.clone(), v) {
        Ok(mut solution) => {
            solution.remove(&context_name);
            let resolves: Vec<String> = solution
                .into_iter()
                .map(|(x, y)| format!("{}/{}/package.py", x, y))
                .collect();
            Ok(resolves)
        }
        Err(e) => Err(e),
    }
}
