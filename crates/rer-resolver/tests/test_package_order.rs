use rer_resolver::package_filter::FilterList;
use rer_resolver::{
    solver_with_packages_ordered, LocalPackages, PackageData, PackageOrderConfig, ResolutionMode,
};
use std::collections::HashMap;

/// Helper to build package data for tests.
fn make_packages(
    specs: Vec<(&str, &str, Vec<&str>, Vec<Vec<&str>>)>,
) -> HashMap<String, HashMap<String, PackageData>> {
    let mut packages: HashMap<String, HashMap<String, PackageData>> = HashMap::new();
    for (name, version, requires, variants) in specs {
        let pkg_data = PackageData {
            requires: requires.into_iter().map(|s| s.to_string()).collect(),
            variants: variants
                .into_iter()
                .map(|v| v.into_iter().map(|s| s.to_string()).collect())
                .collect(),
        };
        packages
            .entry(name.to_string())
            .or_default()
            .insert(version.to_string(), pkg_data);
    }
    packages
}

/// Helper to sort solution for deterministic comparison.
fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

// ---------------------------------------------------------------------------
// Per-family override: lowest for one package, highest for another
// ---------------------------------------------------------------------------

#[test]
fn test_per_family_override_lowest_and_highest() {
    // "alpha" has versions 1.0.0 and 2.0.0.
    // "beta" has versions 1.0.0 and 2.0.0.
    // Default is Highest, but override alpha to Lowest.
    // Expected: alpha/1.0.0, beta/2.0.0
    let packages = make_packages(vec![
        ("alpha", "1.0.0", vec![], vec![]),
        ("alpha", "2.0.0", vec![], vec![]),
        ("beta", "1.0.0", vec![], vec![]),
        ("beta", "2.0.0", vec![], vec![]),
    ]);
    let mut config = PackageOrderConfig::new(ResolutionMode::Highest);
    config.set_family_mode("alpha".to_string(), ResolutionMode::Lowest);

    let mut lp = LocalPackages::from_packages(packages);
    let result = solver_with_packages_ordered(
        vec!["alpha", "beta"],
        &mut lp,
        &FilterList::default(),
        &config,
    )
    .unwrap();
    assert_eq!(
        sorted(result),
        sorted(vec![
            "alpha/1.0.0/package.py".to_string(),
            "beta/2.0.0/package.py".to_string(),
        ])
    );
}

// ---------------------------------------------------------------------------
// Default Lowest: all packages pick lowest version
// ---------------------------------------------------------------------------

#[test]
fn test_default_lowest_mode() {
    let packages = make_packages(vec![
        ("tool", "1.0.0", vec![], vec![]),
        ("tool", "2.0.0", vec![], vec![]),
        ("tool", "3.0.0", vec![], vec![]),
    ]);
    let config = PackageOrderConfig::new(ResolutionMode::Lowest);

    let mut lp = LocalPackages::from_packages(packages);
    let result = solver_with_packages_ordered(
        vec!["tool"],
        &mut lp,
        &FilterList::default(),
        &config,
    )
    .unwrap();
    assert_eq!(
        sorted(result),
        sorted(vec!["tool/1.0.0/package.py".to_string()])
    );
}

// ---------------------------------------------------------------------------
// VersionSplit: pivot correctly prioritizes high side over low side
// ---------------------------------------------------------------------------

#[test]
fn test_version_split_picks_from_high_side() {
    // "tool" has versions 1.0.0, 2.0.0, 3.0.0, 4.0.0.
    // VersionSplit pivot at 3.0.0, first=Lowest (>=3), second=Highest (<3).
    // Should pick 3.0.0 (lowest on high side, which is tried first).
    let packages = make_packages(vec![
        ("tool", "1.0.0", vec![], vec![]),
        ("tool", "2.0.0", vec![], vec![]),
        ("tool", "3.0.0", vec![], vec![]),
        ("tool", "4.0.0", vec![], vec![]),
    ]);
    let config = PackageOrderConfig::new(ResolutionMode::VersionSplit {
        pivot: "3.0.0".try_into().unwrap(),
        first: Box::new(ResolutionMode::Lowest),
        second: Box::new(ResolutionMode::Highest),
    });

    let mut lp = LocalPackages::from_packages(packages);
    let result = solver_with_packages_ordered(
        vec!["tool"],
        &mut lp,
        &FilterList::default(),
        &config,
    )
    .unwrap();
    assert_eq!(
        sorted(result),
        sorted(vec!["tool/3.0.0/package.py".to_string()])
    );
}

// ---------------------------------------------------------------------------
// VersionSplit: falls back to low side when high side is empty
// ---------------------------------------------------------------------------

#[test]
fn test_version_split_falls_back_to_low_side() {
    // "tool" has versions 1.0.0, 2.0.0.
    // VersionSplit pivot at 5.0.0, first=Highest (>=5, empty), second=Highest (<5).
    // Should fall back to 2.0.0 (highest on low side).
    let packages = make_packages(vec![
        ("tool", "1.0.0", vec![], vec![]),
        ("tool", "2.0.0", vec![], vec![]),
    ]);
    let config = PackageOrderConfig::new(ResolutionMode::VersionSplit {
        pivot: "5.0.0".try_into().unwrap(),
        first: Box::new(ResolutionMode::Highest),
        second: Box::new(ResolutionMode::Highest),
    });

    let mut lp = LocalPackages::from_packages(packages);
    let result = solver_with_packages_ordered(
        vec!["tool"],
        &mut lp,
        &FilterList::default(),
        &config,
    )
    .unwrap();
    assert_eq!(
        sorted(result),
        sorted(vec!["tool/2.0.0/package.py".to_string()])
    );
}

// ---------------------------------------------------------------------------
// VersionSplit per-family: one package split, another uses default
// ---------------------------------------------------------------------------

#[test]
fn test_version_split_per_family_with_default() {
    // "python" has versions 2.7.0, 3.7.0, 3.9.0.
    // "maya" has versions 2024.0.0, 2025.0.0.
    // python uses VersionSplit at 3.0.0 (first=Lowest >=3, second=Highest <3).
    // maya uses default Highest.
    // Expected: python/3.7.0 (lowest on high side), maya/2025.0.0 (highest).
    let packages = make_packages(vec![
        ("python", "2.7.0", vec![], vec![]),
        ("python", "3.7.0", vec![], vec![]),
        ("python", "3.9.0", vec![], vec![]),
        ("maya", "2024.0.0", vec![], vec![]),
        ("maya", "2025.0.0", vec![], vec![]),
    ]);
    let mut config = PackageOrderConfig::new(ResolutionMode::Highest);
    config.set_family_mode(
        "python".to_string(),
        ResolutionMode::VersionSplit {
            pivot: "3.0.0".try_into().unwrap(),
            first: Box::new(ResolutionMode::Lowest),
            second: Box::new(ResolutionMode::Highest),
        },
    );

    let mut lp = LocalPackages::from_packages(packages);
    let result = solver_with_packages_ordered(
        vec!["python", "maya"],
        &mut lp,
        &FilterList::default(),
        &config,
    )
    .unwrap();
    assert_eq!(
        sorted(result),
        sorted(vec![
            "python/3.7.0/package.py".to_string(),
            "maya/2025.0.0/package.py".to_string(),
        ])
    );
}
