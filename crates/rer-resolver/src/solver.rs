use crate::candidate_selector::CandidateList;
use crate::LocalPackages;
use pubgrub::error::PubGrubError;
use pubgrub::range::Range;
use pubgrub::report::{DefaultStringReporter, Reporter};
use pubgrub::solver::{resolve, OfflineDependencyProvider};
use rer_version::requirement::Requirement;
use rer_version::{requirement::Requirements, RerVersion};
use std::collections::HashMap;
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
    local_packages: &LocalPackages,
    dependencies: Requirements,
    cache_requierements: &Arc<Mutex<HashMap<Requirement, bool>>>,
) {
    for dependency in dependencies {
        // Acquire the mutex
        let mut cache_requi = cache_requierements.lock().unwrap();
        if cache_requi.contains_key(&dependency) {
            continue;
        } else {
            cache_requi.insert(dependency.clone(), true);
        }
        // Drop the mutex to be sure it is not poison
        drop(cache_requi);
        let package_name = dependency.get_name().to_string();
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
            let dependencies =
                local_packages.get_dependencies(&package_name, &candidate.to_string());
            drop(dependency_provider_guard);
            recursive(
                dependency_provider,
                local_packages,
                dependencies,
                cache_requierements,
            );
        }
    }
}

pub fn solver(requirements_str: Vec<&str>, packages: LocalPackages) -> Vec<String> {
    let dependency_provider: Arc<Mutex<OfflineDependencyProvider<String, RerVersion>>> = Arc::new(
        Mutex::new(OfflineDependencyProvider::<String, RerVersion>::new()),
    );
    let cache_requierements: Arc<Mutex<HashMap<Requirement, bool>>> =
        Arc::new(Mutex::new(HashMap::new()));
    // Create a uuid to represent the current request
    let context_name = Uuid::new_v4().to_string();
    // Transform the list of str into a list of Requirement and merge if possible
    let dependencies: Requirements = Requirements::from_str(requirements_str).merge();
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
        &packages,
        dependencies,
        &cache_requierements,
    );
    let p: OfflineDependencyProvider<String, RerVersion> =
        dependency_provider.lock().unwrap().clone();
    let solution = match resolve(&p, context_name, v) {
        Ok(solution) => solution,
        Err(PubGrubError::NoSolution(mut derivation_tree)) => {
            derivation_tree.collapse_no_versions();
            eprintln!("{}", DefaultStringReporter::report(&derivation_tree));
            panic!("No solution found")
        }
        Err(err) => panic!("{:?}", err),
    };
    solution
        .into_iter()
        .map(|(name, version)| format!("{}=={}", name, version.to_string()))
        .collect()
}
