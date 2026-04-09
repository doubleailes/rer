use regex::Regex;
use rer_resolver::package_filter::{FilterList, GlobFilter, RegexFilter};
use rer_resolver::{solver_with_packages_filtered, LocalPackages, PackageData};
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
// Filter excludes highest version → solver picks next best
// ---------------------------------------------------------------------------

#[test]
fn test_filter_excludes_version_solver_picks_next() {
    // Two versions of "tool": 2.0.0 and 1.0.0.
    // Without filters, solver picks 2.0.0 (highest).
    // With a regex filter excluding version 2.*, solver picks 1.0.0.
    let packages = make_packages(vec![
        ("tool", "1.0.0", vec![], vec![]),
        ("tool", "2.0.0", vec![], vec![]),
    ]);

    // Without filters: picks 2.0.0
    let mut lp = LocalPackages::from_packages(packages.clone());
    let result =
        solver_with_packages_filtered(vec!["tool"], &mut lp, &FilterList::default()).unwrap();
    assert_eq!(
        sorted(result),
        sorted(vec!["tool/2.0.0/package.py".to_string()])
    );

    // With regex filter excluding version 2.*: picks 1.0.0
    let mut filters = FilterList::new();
    filters.add(Box::new(RegexFilter {
        family: Some(Regex::new("^tool$").unwrap()),
        version: Some(Regex::new("^2\\.").unwrap()),
    }));
    let mut lp = LocalPackages::from_packages(packages);
    let result = solver_with_packages_filtered(vec!["tool"], &mut lp, &filters).unwrap();
    assert_eq!(
        sorted(result),
        sorted(vec!["tool/1.0.0/package.py".to_string()])
    );
}

// ---------------------------------------------------------------------------
// Glob filter excludes a package family entirely
// ---------------------------------------------------------------------------

#[test]
fn test_glob_filter_excludes_family() {
    // "internal_tool" depends on "lib", but "internal_tool" is excluded by glob.
    // Solver should fail to resolve "internal_tool".
    let packages = make_packages(vec![
        ("lib", "1.0.0", vec![], vec![]),
        ("internal_tool", "1.0.0", vec!["lib-1"], vec![]),
    ]);
    let mut filters = FilterList::new();
    filters.add(Box::new(GlobFilter::new("internal_*").unwrap()));
    let mut lp = LocalPackages::from_packages(packages);
    let result = solver_with_packages_filtered(vec!["internal_tool-1.0.0"], &mut lp, &filters);
    assert!(
        result.is_err(),
        "Should fail when the requested package is excluded by a filter"
    );
}

// ---------------------------------------------------------------------------
// Filter excludes a dependency → solver falls back to alternative
// ---------------------------------------------------------------------------

#[test]
fn test_filter_excludes_dependency_version() {
    // "app" requires "lib-1". lib has versions 1.0.0 and 1.1.0.
    // Filter excludes lib/1.1.0 → solver picks lib/1.0.0.
    let packages = make_packages(vec![
        ("lib", "1.0.0", vec![], vec![]),
        ("lib", "1.1.0", vec![], vec![]),
        ("app", "1.0.0", vec!["lib-1"], vec![]),
    ]);
    let mut filters = FilterList::new();
    filters.add(Box::new(RegexFilter {
        family: Some(Regex::new("^lib$").unwrap()),
        version: Some(Regex::new("^1\\.1").unwrap()),
    }));
    let mut lp = LocalPackages::from_packages(packages);
    let result = solver_with_packages_filtered(vec!["app-1.0.0"], &mut lp, &filters).unwrap();
    let result = sorted(result);
    assert_eq!(
        result,
        sorted(vec![
            "app/1.0.0/package.py".to_string(),
            "lib/1.0.0/package.py".to_string(),
        ])
    );
}

// ---------------------------------------------------------------------------
// Multiple filters compose with OR semantics
// ---------------------------------------------------------------------------

#[test]
fn test_multiple_filters_or_semantics() {
    // Two packages: "alpha" and "beta", each with version 1.0.0.
    // Filter 1 excludes "alpha", filter 2 excludes "beta".
    // Requesting both should fail since both are excluded.
    let packages = make_packages(vec![
        ("alpha", "1.0.0", vec![], vec![]),
        ("beta", "1.0.0", vec![], vec![]),
    ]);
    let mut filters = FilterList::new();
    filters.add(Box::new(GlobFilter::new("alpha").unwrap()));
    filters.add(Box::new(GlobFilter::new("beta").unwrap()));
    let mut lp = LocalPackages::from_packages(packages);
    let result =
        solver_with_packages_filtered(vec!["alpha-1.0.0", "beta-1.0.0"], &mut lp, &filters);
    assert!(
        result.is_err(),
        "Should fail when all versions of requested packages are excluded"
    );
}

// ---------------------------------------------------------------------------
// Empty filter list has no effect
// ---------------------------------------------------------------------------

#[test]
fn test_empty_filter_list_no_effect() {
    let packages = make_packages(vec![
        ("python", "3.9.0", vec![], vec![]),
        ("foo", "1.0.0", vec!["python-3"], vec![]),
    ]);
    let mut lp = LocalPackages::from_packages(packages);
    let result =
        solver_with_packages_filtered(vec!["foo-1.0.0"], &mut lp, &FilterList::default()).unwrap();
    let result = sorted(result);
    assert_eq!(
        result,
        sorted(vec![
            "foo/1.0.0/package.py".to_string(),
            "python/3.9.0/package.py".to_string(),
        ])
    );
}

// ---------------------------------------------------------------------------
// Filter with multi-variant: excluded version forces fallback
// ---------------------------------------------------------------------------

#[test]
fn test_filter_with_multi_variant() {
    // tool v2.0.0 has multi-variant: [maya-2024], [maya-2025]
    // tool v1.0.0 has multi-variant: [maya-2024], [maya-2025]
    // Filter excludes tool/2.0.0 → solver picks tool/1.0.0
    let packages = make_packages(vec![
        ("maya", "2024.0.0", vec![], vec![]),
        ("maya", "2025.0.0", vec![], vec![]),
        (
            "tool",
            "1.0.0",
            vec![],
            vec![vec!["maya-2024"], vec!["maya-2025"]],
        ),
        (
            "tool",
            "2.0.0",
            vec![],
            vec![vec!["maya-2024"], vec!["maya-2025"]],
        ),
    ]);
    let mut filters = FilterList::new();
    filters.add(Box::new(RegexFilter {
        family: Some(Regex::new("^tool$").unwrap()),
        version: Some(Regex::new("^2\\.").unwrap()),
    }));
    let mut lp = LocalPackages::from_packages(packages);
    let result = solver_with_packages_filtered(vec!["tool"], &mut lp, &filters).unwrap();
    let result = sorted(result);
    // Should pick tool/1.0.0 with the highest compatible variant (maya-2025)
    assert!(result.contains(&"tool/1.0.0/package.py".to_string()));
    assert!(!result.contains(&"tool/2.0.0/package.py".to_string()));
}
