use rer_version::RerVersion;
use std::collections::HashMap;
use version_ranges::Ranges;

pub struct CandidateList(Vec<RerVersion>);

impl CandidateList {
    pub fn new(mut candidat_list: Vec<RerVersion>) -> Self {
        candidat_list.sort();
        CandidateList(candidat_list)
    }
    pub fn sort(&mut self) {
        self.0.sort();
    }
    pub fn from_vec_str(v: Vec<&str>) -> Self {
        let mut list: Vec<RerVersion> = v
            .into_iter()
            .map(|x| x.try_into().expect("can't convert"))
            .collect();
        list.sort();
        CandidateList(list)
    }
    pub fn find_candidate(
        &self,
        range: &Ranges<RerVersion>,
        strategy_mode: &ResolutionMode,
    ) -> Option<RerVersion> {
        match strategy_mode {
            ResolutionMode::Highest => self.0.iter().rev().find(|&x| range.contains(x)).cloned(),
            ResolutionMode::Lowest => self.0.iter().find(|&x| range.contains(x)).cloned(),
            ResolutionMode::VersionSplit {
                pivot,
                first,
                second,
            } => {
                // Try the "first" strategy on versions >= pivot
                let high_range = range.intersection(&Ranges::higher_than(pivot.clone()));
                let high_candidate = if !high_range.is_empty() {
                    self.find_candidate(&high_range, first)
                } else {
                    None
                };
                if high_candidate.is_some() {
                    return high_candidate;
                }
                // Fall back to the "second" strategy on versions < pivot
                let low_range = range.intersection(&Ranges::strictly_lower_than(pivot.clone()));
                if !low_range.is_empty() {
                    self.find_candidate(&low_range, second)
                } else {
                    None
                }
            }
        }
    }
    pub fn find_candidates(&self, range: &Ranges<RerVersion>) -> Vec<&RerVersion> {
        self.0.iter().filter(|&x| range.contains(x)).collect()
    }
}

/// Select the best version from an arbitrary iterator without requiring a pre-sorted input.
///
/// For `Highest`/`Lowest`, performs a single linear scan using `max()`/`min()`.
/// For `VersionSplit`, collects matching versions once and recurses on sub-slices.
/// Only the single chosen version is cloned.
///
/// Use this in hot paths (e.g., `DependencyProvider::choose_version`) instead of
/// constructing a `CandidateList` just to call `find_candidate` once.
pub fn find_best_candidate<'a, I>(
    versions: I,
    range: &Ranges<RerVersion>,
    mode: &ResolutionMode,
) -> Option<RerVersion>
where
    I: Iterator<Item = &'a RerVersion>,
{
    match mode {
        ResolutionMode::Highest => versions.filter(|v| range.contains(v)).max().cloned(),
        ResolutionMode::Lowest => versions.filter(|v| range.contains(v)).min().cloned(),
        ResolutionMode::VersionSplit {
            pivot,
            first,
            second,
        } => {
            let high_range = range.intersection(&Ranges::higher_than(pivot.clone()));
            let low_range = range.intersection(&Ranges::strictly_lower_than(pivot.clone()));
            // Collect matching versions once to allow two passes without re-iterating.
            let matching: Vec<&RerVersion> = versions.filter(|v| range.contains(v)).collect();
            if !high_range.is_empty() {
                let candidate = find_best_candidate(matching.iter().copied(), &high_range, first);
                if candidate.is_some() {
                    return candidate;
                }
            }
            if !low_range.is_empty() {
                find_best_candidate(matching.iter().copied(), &low_range, second)
            } else {
                None
            }
        }
    }
}

/// Resolution strategy for selecting package versions.
#[derive(Debug, Default, Clone)]
pub enum ResolutionMode {
    /// Resolve the highest compatible version of each package.
    #[default]
    Highest,
    /// Resolve the lowest compatible version of each package.
    Lowest,
    /// Split versions around a pivot point.
    ///
    /// Versions >= pivot are tried first using the `first` strategy,
    /// then versions < pivot are tried using the `second` strategy.
    /// This mirrors rez's `VersionSplitPackageOrder`.
    VersionSplit {
        /// The version to split around.
        pivot: RerVersion,
        /// Strategy for versions >= pivot (tried first).
        first: Box<ResolutionMode>,
        /// Strategy for versions < pivot (tried second).
        second: Box<ResolutionMode>,
    },
}

/// Per-family package ordering configuration.
///
/// Allows specifying a default resolution mode and per-family overrides,
/// mirroring rez's `PackageOrder` and `PerFamilyOrder` strategies.
///
/// # Examples
///
/// ```
/// use rer_resolver::{PackageOrderConfig, ResolutionMode};
///
/// let mut config = PackageOrderConfig::new(ResolutionMode::Highest);
/// config.set_family_mode("python".to_string(), ResolutionMode::Lowest);
///
/// assert!(matches!(config.mode_for("python"), ResolutionMode::Lowest));
/// assert!(matches!(config.mode_for("maya"), ResolutionMode::Highest));
/// ```
#[derive(Debug, Clone, Default)]
pub struct PackageOrderConfig {
    /// The default resolution mode for all packages.
    pub default: ResolutionMode,
    /// Per-family resolution mode overrides.
    pub per_family: HashMap<String, ResolutionMode>,
}

impl PackageOrderConfig {
    /// Create a new `PackageOrderConfig` with the given default mode and no per-family overrides.
    pub fn new(default: ResolutionMode) -> Self {
        Self {
            default,
            per_family: HashMap::new(),
        }
    }

    /// Set a per-family resolution mode override.
    pub fn set_family_mode(&mut self, family: String, mode: ResolutionMode) {
        self.per_family.insert(family, mode);
    }

    /// Get the resolution mode for a given package family.
    ///
    /// Returns the per-family override if one exists, otherwise the default.
    pub fn mode_for(&self, family: &str) -> &ResolutionMode {
        self.per_family.get(family).unwrap_or(&self.default)
    }
}

#[test]
fn test_candidate_list() {
    let list = CandidateList::from_vec_str(vec!["1.0.0", "1.1.0", "1.2.0"]);
    let v1: RerVersion = "1.0.0".try_into().unwrap();
    let v2: RerVersion = "1.2.0".try_into().unwrap();
    let range = Ranges::between(v1, v2);
    let v3: RerVersion = "1.1.0".try_into().unwrap();
    assert_eq!(
        list.find_candidate(&range, &ResolutionMode::Highest),
        Some(v3)
    );
    let v3: RerVersion = "1.0.0".try_into().unwrap();
    assert_eq!(
        list.find_candidate(&range, &ResolutionMode::Lowest),
        Some(v3)
    );
}

#[test]
fn test_candidates_list() {
    let list = CandidateList::from_vec_str(vec!["1.0.0", "1.1.0", "1.1.1", "1.2.0"]);
    let start: RerVersion = "1.0.0".try_into().unwrap();
    let end: RerVersion = "1.2.0".try_into().unwrap();
    let range = Ranges::between(start, end);
    let v1: RerVersion = "1.0.0".try_into().unwrap();
    let v3: RerVersion = "1.1.0".try_into().unwrap();
    let v4: RerVersion = "1.1.1".try_into().unwrap();
    let mut results = vec![&v1, &v3, &v4];
    results.sort();
    assert_eq!(list.find_candidates(&range), results);
    let list =
        CandidateList::from_vec_str(vec!["4.8.6.m1", "4.8.6.m2", "5.12.6", "5.6.1", "4.8.6.m3"]);
    let v1: RerVersion = "4.8.6".try_into().unwrap();
    let range = Ranges::between(v1.clone(), v1.bump());
    let v2: RerVersion = "4.8.6.m3".try_into().unwrap();
    assert_eq!(
        list.find_candidate(&range, &ResolutionMode::Highest),
        Some(v2)
    );
}

#[test]
fn test_version_split_prefers_high_side() {
    // Versions: 1.0.0, 2.0.0, 3.0.0, 4.0.0
    // Pivot at 3.0.0: first=Highest (>=3), second=Highest (<3)
    // Should pick 4.0.0 (highest on the high side)
    let list = CandidateList::from_vec_str(vec!["1.0.0", "2.0.0", "3.0.0", "4.0.0"]);
    let range = Ranges::full();
    let pivot: RerVersion = "3.0.0".try_into().unwrap();
    let mode = ResolutionMode::VersionSplit {
        pivot,
        first: Box::new(ResolutionMode::Highest),
        second: Box::new(ResolutionMode::Highest),
    };
    assert_eq!(
        list.find_candidate(&range, &mode),
        Some("4.0.0".try_into().unwrap())
    );
}

#[test]
fn test_version_split_falls_back_to_low_side() {
    // Versions: 1.0.0, 2.0.0
    // Pivot at 3.0.0: first=Highest (>=3, empty), second=Highest (<3)
    // Should fall back to 2.0.0 (highest on the low side)
    let list = CandidateList::from_vec_str(vec!["1.0.0", "2.0.0"]);
    let range = Ranges::full();
    let pivot: RerVersion = "3.0.0".try_into().unwrap();
    let mode = ResolutionMode::VersionSplit {
        pivot,
        first: Box::new(ResolutionMode::Highest),
        second: Box::new(ResolutionMode::Highest),
    };
    assert_eq!(
        list.find_candidate(&range, &mode),
        Some("2.0.0".try_into().unwrap())
    );
}

#[test]
fn test_version_split_lowest_on_high_side() {
    // Versions: 1.0.0, 2.0.0, 3.0.0, 4.0.0
    // Pivot at 2.5.0: first=Lowest (>=2.5), second=Lowest (<2.5)
    // Should pick 3.0.0 (lowest on the high side)
    let list = CandidateList::from_vec_str(vec!["1.0.0", "2.0.0", "3.0.0", "4.0.0"]);
    let range = Ranges::full();
    let pivot: RerVersion = "2.5.0".try_into().unwrap();
    let mode = ResolutionMode::VersionSplit {
        pivot: pivot.clone(),
        first: Box::new(ResolutionMode::Lowest),
        second: Box::new(ResolutionMode::Lowest),
    };
    assert_eq!(
        list.find_candidate(&range, &mode),
        Some("3.0.0".try_into().unwrap())
    );
}

#[test]
fn test_package_order_config_default() {
    let config = PackageOrderConfig::new(ResolutionMode::Highest);
    assert!(matches!(
        config.mode_for("anything"),
        ResolutionMode::Highest
    ));
}

#[test]
fn test_package_order_config_per_family() {
    let mut config = PackageOrderConfig::new(ResolutionMode::Highest);
    config.set_family_mode("python".to_string(), ResolutionMode::Lowest);
    assert!(matches!(config.mode_for("python"), ResolutionMode::Lowest));
    assert!(matches!(config.mode_for("maya"), ResolutionMode::Highest));
}
