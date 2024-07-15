use pubgrub::range::Range;
use pubgrub::solver::{DependencyConstraints, DependencyProvider};
use pubgrub::type_aliases::Map;
use rer_version::RerVersion;
use std::collections::BTreeMap;
use std::fmt::Debug;
use pubgrub::solver::Dependencies;
use std::error::Error;

#[derive(Debug, Clone, Default)]
pub struct RerDependencyProvider {
    dependencies: Map<String, BTreeMap<RerVersion, DependencyConstraints<String, RerVersion>>>,
    // For examplae python is constained by Maya at a certaun to a specific version ( of python )
    weak_refeence: Map<String, Map<String, DependencyConstraints<RerVersion, RerVersion>>>,
}

impl RerDependencyProvider {
    pub fn add_dependencies(
        &mut self,
        package: String,
        version: RerVersion,
        dependencies: Vec<(String, Range<RerVersion>)>,
    ) {
        let package_deps = dependencies.into_iter().collect();
        *self
            .dependencies
            .entry(package)
            .or_default()
            .entry(version)
            .or_default() = package_deps;
    }
}

impl DependencyProvider<String, RerVersion> for RerDependencyProvider {
    fn choose_package_version<
        T: std::borrow::Borrow<String>,
        U: std::borrow::Borrow<Range<RerVersion>>,
    >(
        &self,
        potential_packages: impl Iterator<Item = (T, U)>,
    ) -> Result<(T, Option<RerVersion>), Box<dyn std::error::Error>> {
        unimplemented!()
    }
    fn get_dependencies(
        &self,
        package: &String,
        version: &RerVersion,
    ) ->  Result<Dependencies<String, RerVersion>, Box<dyn Error>> {
        unimplemented!()
    }
}
