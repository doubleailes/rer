use crate::candidate_selector::CandidateList;
use crate::local_package::PackageData;
use crate::LocalPackages;
use pubgrub::{resolve, OfflineDependencyProvider, PubGrubError};
use rer_version::{Requirement, Requirements, RerVersion};
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use uuid::Uuid;
use version_ranges::Ranges;

/// Naming convention for variant selector packages: `{package_name}[v]`
fn variant_selector_name(package_name: &str) -> String {
    format!("{}[v]", package_name)
}

/// Create a variant-encoded version: `{base_version}.{variant_index}`
fn variant_version(base: &RerVersion, variant_index: usize) -> RerVersion {
    let s = format!("{}.{}", base, variant_index);
    RerVersion::try_from(s.as_str())
        .unwrap_or_else(|_| panic!("Failed to create variant version {}", s))
}

/// Returns true if a package name is a variant selector (contains `[v]` suffix).
fn is_variant_selector(name: &str) -> bool {
    name.ends_with("[v]")
}

fn check_version<'a>(
    dependency_provider: &'a Arc<Mutex<OfflineDependencyProvider<String, Ranges<RerVersion>>>>,
    package_name: &'a String,
    version: &'a RerVersion,
) -> bool {
    let versions_package: Vec<RerVersion> =
        match dependency_provider.lock().unwrap().versions(package_name) {
            Some(versions_package) => versions_package.into_iter().cloned().collect(),
            None => Vec::new(),
        };
    versions_package.contains(version)
}

/// Register a multi-variant package into the dependency provider.
///
/// For a package `foo/1.0.0` with requires=["python-3"] and variants=[["maya-2024"], ["maya-2025"]]:
/// - Registers `foo/1.0.0` → deps: python-3, foo[v] ∈ [1.0.0, 1.0.0_)
/// - Registers `foo[v]/1.0.0.0` → deps: maya-2024
/// - Registers `foo[v]/1.0.0.1` → deps: maya-2025
///
/// Pubgrub picks one variant version of `foo[v]` via version selection and backtracking.
fn register_multi_variant(
    dependency_provider: &Arc<Mutex<OfflineDependencyProvider<String, Ranges<RerVersion>>>>,
    package_name: &str,
    version: &RerVersion,
    package_data: &PackageData,
) {
    let selector_name = variant_selector_name(package_name);

    // Build base package deps: requires + dependency on variant selector
    let requires_reqs = Requirements::from(
        package_data
            .requires
            .iter()
            .map(|s| s.as_str())
            .collect(),
    );
    let mut base_deps: Vec<(String, Ranges<RerVersion>)> = requires_reqs.to_pubgrub();
    // Add dependency on variant selector with range [version, version.bump())
    base_deps.push((
        selector_name.clone(),
        Ranges::between(version.clone(), version.bump()),
    ));

    // Register base package
    let mut dp = dependency_provider.lock().unwrap();
    dp.add_dependencies(package_name.to_string(), version.clone(), base_deps);

    // Register each variant as a version of the selector package
    for (i, variant_deps) in package_data.variants.iter().enumerate() {
        let v_version = variant_version(version, i);
        let variant_reqs =
            Requirements::from(variant_deps.iter().map(|s| s.as_str()).collect());
        let variant_pubgrub: Vec<(String, Ranges<RerVersion>)> = variant_reqs.to_pubgrub();
        dp.add_dependencies(selector_name.clone(), v_version, variant_pubgrub);
    }
}

fn recursive(
    dependency_provider: &Arc<Mutex<OfflineDependencyProvider<String, Ranges<RerVersion>>>>,
    local_packages: &mut LocalPackages,
    dependencies: Requirements,
    cache_requierements: &Arc<Mutex<HashSet<Requirement>>>,
    weak_references: &Arc<Mutex<HashMap<String, Ranges<RerVersion>>>>,
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
            let range = dependency.get_version_range().unwrap_or(Ranges::full());
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
        let range = dependency.get_version_range().unwrap_or(Ranges::full());
        let candidates = candidates.find_candidates(&range);
        for candidate in candidates {
            if check_version(dependency_provider, &package_name, candidate) {
                continue;
            }

            // Check for multi-variant package data
            let pkg_data = local_packages.get_package_data(&package_name, &candidate.to_string());
            if let Some(ref data) = pkg_data {
                if data.is_multi_variant() {
                    // Register using variant selector encoding
                    register_multi_variant(
                        dependency_provider,
                        &package_name,
                        candidate,
                        data,
                    );

                    // Recursively explore requires deps
                    let requires = Requirements::from(
                        data.requires.iter().map(|s| s.as_str()).collect(),
                    );
                    if !requires.is_empty() {
                        recursive(
                            dependency_provider,
                            local_packages,
                            requires,
                            cache_requierements,
                            weak_references,
                        );
                    }

                    // Recursively explore ALL variant deps (to build complete graph)
                    for variant_deps in &data.variants {
                        let variant_reqs = Requirements::from(
                            variant_deps.iter().map(|s| s.as_str()).collect(),
                        );
                        if !variant_reqs.is_empty() {
                            recursive(
                                dependency_provider,
                                local_packages,
                                variant_reqs,
                                cache_requierements,
                                weak_references,
                            );
                        }
                    }
                    continue;
                }
            }

            // No variants or single variant: use combined dependencies
            let dependencies =
                local_packages.get_dependencies(&package_name, &candidate.to_string());
            let dependencies_pubgrub: Vec<(String, Ranges<RerVersion>)> = dependencies.to_pubgrub();
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
) -> Result<Vec<String>, PubGrubError<OfflineDependencyProvider<String, Ranges<RerVersion>>>> {
    let mut packages = LocalPackages::lazy_paths(paths);
    solver_with_packages(requirements_str, &mut packages)
}

/// Solve with a pre-built `LocalPackages` instance. Supports variant-aware package data.
pub fn solver_with_packages(
    requirements_str: Vec<&str>,
    packages: &mut LocalPackages,
) -> Result<Vec<String>, PubGrubError<OfflineDependencyProvider<String, Ranges<RerVersion>>>> {
    let dependency_provider: Arc<Mutex<OfflineDependencyProvider<String, Ranges<RerVersion>>>> = Arc::new(
        Mutex::new(OfflineDependencyProvider::<String, Ranges<RerVersion>>::new()),
    );
    let cache_requierements: Arc<Mutex<HashSet<Requirement>>> =
        Arc::new(Mutex::new(HashSet::new()));
    let weak_references: Arc<Mutex<HashMap<String, Ranges<RerVersion>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    // Create a uuid to represent the current request
    let context_name = Uuid::new_v4().to_string();
    // Transform the list of str into a list of Requirement and merge if possible
    let dependencies: Requirements = Requirements::from(requirements_str).merge();
    let mut new_deps = Requirements::empty();
    for dependency in dependencies {
        if dependency.is_weak_ref() {
            let name = dependency.get_name().to_string();
            let range = dependency.get_version_range().unwrap_or(Ranges::full());
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
    let dependencies_pubgrub: Vec<(String, Ranges<RerVersion>)> = dependencies.to_pubgrub();
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
        packages,
        dependencies,
        &cache_requierements,
        &weak_references,
    );
    let p: OfflineDependencyProvider<String, Ranges<RerVersion>> =
        dependency_provider.lock().unwrap().clone();
    match resolve(&p, context_name.clone(), v) {
        Ok(mut solution) => {
            solution.remove(&context_name);
            let resolves: Vec<String> = solution
                .into_iter()
                // Filter out variant selector packages from the output
                .filter(|(name, _)| !is_variant_selector(name))
                .map(|(x, y)| format!("{}/{}/package.py", x, y))
                .collect();
            Ok(resolves)
        }
        Err(e) => Err(e),
    }
}
