use rer_resolver::{solver_with_packages, LocalPackages, PackageData};
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
// No-variant packages: existing behavior preserved
// ---------------------------------------------------------------------------

#[test]
fn test_no_variant_package() {
    let packages = make_packages(vec![
        ("python", "3.9.0", vec![], vec![]),
        ("foo", "1.0.0", vec!["python-3"], vec![]),
    ]);
    let mut lp = LocalPackages::from_packages(packages);
    let result = solver_with_packages(vec!["foo-1.0.0"], &mut lp).unwrap();
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
// Single-variant packages: variant deps merged with requires
// ---------------------------------------------------------------------------

#[test]
fn test_single_variant_package() {
    let packages = make_packages(vec![
        ("python", "3.9.0", vec![], vec![]),
        ("maya", "2024.0.0", vec![], vec![]),
        (
            "maya_utils",
            "1.0.0",
            vec!["python-3"],
            vec![vec!["maya-2024"]],
        ),
    ]);
    let mut lp = LocalPackages::from_packages(packages);
    let result = solver_with_packages(vec!["maya_utils-1.0.0"], &mut lp).unwrap();
    let result = sorted(result);
    assert_eq!(
        result,
        sorted(vec![
            "maya_utils/1.0.0/package.py".to_string(),
            "python/3.9.0/package.py".to_string(),
            "maya/2024.0.0/package.py".to_string(),
        ])
    );
}

// ---------------------------------------------------------------------------
// Multi-variant: 2 variants, solver picks one
// ---------------------------------------------------------------------------

#[test]
fn test_multi_variant_two_variants() {
    let packages = make_packages(vec![
        ("python", "3.9.0", vec![], vec![]),
        ("maya", "2024.0.0", vec![], vec![]),
        ("maya", "2025.0.0", vec![], vec![]),
        (
            "maya_utils",
            "1.0.0",
            vec!["python-3"],
            vec![vec!["maya-2024"], vec!["maya-2025"]],
        ),
    ]);
    let mut lp = LocalPackages::from_packages(packages);
    let result = solver_with_packages(vec!["maya_utils-1.0.0"], &mut lp).unwrap();
    let result = sorted(result);

    // Solver should pick one variant. Since pubgrub tries highest first,
    // variant 1 (maya-2025) should be picked.
    assert_eq!(
        result,
        sorted(vec![
            "maya_utils/1.0.0/package.py".to_string(),
            "python/3.9.0/package.py".to_string(),
            "maya/2025.0.0/package.py".to_string(),
        ])
    );
}

// ---------------------------------------------------------------------------
// Acceptance Criterion: 3+ variants, only one compatible with request
// ---------------------------------------------------------------------------

#[test]
fn test_multi_variant_three_variants_one_compatible() {
    // maya_utils has 3 variants: maya-2023, maya-2024, maya-2025
    // But only maya-2024 is available, so variant 1 should be picked.
    let packages = make_packages(vec![
        ("python", "3.9.0", vec![], vec![]),
        ("maya", "2024.0.0", vec![], vec![]),
        // maya-2023 and maya-2025 are NOT available
        (
            "maya_utils",
            "1.0.0",
            vec!["python-3"],
            vec![
                vec!["maya-2023"],
                vec!["maya-2024"],
                vec!["maya-2025"],
            ],
        ),
    ]);
    let mut lp = LocalPackages::from_packages(packages);
    let result = solver_with_packages(vec!["maya_utils-1.0.0"], &mut lp).unwrap();
    let result = sorted(result);

    // Only variant 1 (maya-2024) can be satisfied
    assert_eq!(
        result,
        sorted(vec![
            "maya_utils/1.0.0/package.py".to_string(),
            "python/3.9.0/package.py".to_string(),
            "maya/2024.0.0/package.py".to_string(),
        ])
    );
}

#[test]
fn test_multi_variant_four_variants_constrained() {
    // Package with 4 variants, but the root request constrains maya version
    // to 2024+, so only variant 1 and 2 are candidates.
    // maya-2025 is available, so variant 2 (highest compatible) is picked.
    let packages = make_packages(vec![
        ("python", "3.9.0", vec![], vec![]),
        ("maya", "2024.0.0", vec![], vec![]),
        ("maya", "2025.0.0", vec![], vec![]),
        (
            "tool",
            "1.0.0",
            vec!["python-3"],
            vec![
                vec!["maya-2022"],
                vec!["maya-2024"],
                vec!["maya-2025"],
                vec!["maya-2026"],
            ],
        ),
    ]);
    let mut lp = LocalPackages::from_packages(packages);
    // Request tool and constrain maya to 2024+
    let result = solver_with_packages(vec!["tool-1.0.0", "maya-2024+"], &mut lp).unwrap();
    let result = sorted(result);

    // Variant 2 (maya-2025) or variant 1 (maya-2024) should be picked,
    // highest compatible first → variant 2 (maya-2025)
    assert_eq!(
        result,
        sorted(vec![
            "tool/1.0.0/package.py".to_string(),
            "python/3.9.0/package.py".to_string(),
            "maya/2025.0.0/package.py".to_string(),
        ])
    );
}

// ---------------------------------------------------------------------------
// Acceptance Criterion: two packages, 2 variants each, cross-compatible
// ---------------------------------------------------------------------------

#[test]
fn test_two_packages_cross_compatible_variants() {
    // renderer has 2 variants: [maya-2024] and [maya-2025]
    // shader has 2 variants: [maya-2024] and [maya-2025]
    // Both maya versions available. Root requests renderer + shader.
    // Solver should find a consistent combination (both pick same maya).
    let packages = make_packages(vec![
        ("python", "3.9.0", vec![], vec![]),
        ("maya", "2024.0.0", vec![], vec![]),
        ("maya", "2025.0.0", vec![], vec![]),
        (
            "renderer",
            "1.0.0",
            vec!["python-3"],
            vec![vec!["maya-2024"], vec!["maya-2025"]],
        ),
        (
            "shader",
            "2.0.0",
            vec!["python-3"],
            vec![vec!["maya-2024"], vec!["maya-2025"]],
        ),
    ]);
    let mut lp = LocalPackages::from_packages(packages);
    let result = solver_with_packages(vec!["renderer-1.0.0", "shader-2.0.0"], &mut lp).unwrap();
    let result = sorted(result);

    // Both should pick maya-2025 (highest variant, since both maya versions exist)
    assert_eq!(
        result,
        sorted(vec![
            "renderer/1.0.0/package.py".to_string(),
            "shader/2.0.0/package.py".to_string(),
            "python/3.9.0/package.py".to_string(),
            "maya/2025.0.0/package.py".to_string(),
        ])
    );
}

#[test]
fn test_two_packages_only_one_cross_compatible_combination() {
    // renderer has 2 variants: [maya-2024, opengl-4] and [maya-2025, vulkan-1]
    // shader has 2 variants: [maya-2024, vulkan-1] and [maya-2025, opengl-4]
    //
    // Available: maya-2024, maya-2025, opengl-4.0.0, vulkan-1.0.0
    //
    // Cross-compatibility:
    // - renderer[0] + shader[0] → maya-2024, opengl-4, vulkan-1 ← compatible!
    // - renderer[0] + shader[1] → maya conflict (2024 vs 2025)
    // - renderer[1] + shader[0] → maya conflict (2025 vs 2024)
    // - renderer[1] + shader[1] → maya-2025, vulkan-1, opengl-4 ← compatible!
    //
    // Since pubgrub tries highest variant first, it should pick
    // renderer[1] + shader[1] (maya-2025 combination).
    let packages = make_packages(vec![
        ("maya", "2024.0.0", vec![], vec![]),
        ("maya", "2025.0.0", vec![], vec![]),
        ("opengl", "4.0.0", vec![], vec![]),
        ("vulkan", "1.0.0", vec![], vec![]),
        (
            "renderer",
            "1.0.0",
            vec![],
            vec![
                vec!["maya-2024", "opengl-4"],
                vec!["maya-2025", "vulkan-1"],
            ],
        ),
        (
            "shader",
            "2.0.0",
            vec![],
            vec![
                vec!["maya-2024", "vulkan-1"],
                vec!["maya-2025", "opengl-4"],
            ],
        ),
    ]);
    let mut lp = LocalPackages::from_packages(packages);
    let result = solver_with_packages(vec!["renderer-1.0.0", "shader-2.0.0"], &mut lp).unwrap();
    let result = sorted(result);

    // Should find a valid combination. Both renderer[1]+shader[1] and
    // renderer[0]+shader[0] are valid. Pubgrub will find one of them.
    // The result should include exactly one maya version and the correct deps.
    assert!(result.contains(&"renderer/1.0.0/package.py".to_string()));
    assert!(result.contains(&"shader/2.0.0/package.py".to_string()));

    // Check that exactly one maya version is in the solution
    let maya_entries: Vec<&String> = result
        .iter()
        .filter(|s| s.starts_with("maya/"))
        .collect();
    assert_eq!(maya_entries.len(), 1, "Should have exactly one maya version");

    // Verify opengl and vulkan are present (both combinations need both)
    assert!(result.contains(&"opengl/4.0.0/package.py".to_string()));
    assert!(result.contains(&"vulkan/1.0.0/package.py".to_string()));
}

// ---------------------------------------------------------------------------
// Edge case: multi-variant where no variant is satisfiable → error
// ---------------------------------------------------------------------------

#[test]
fn test_multi_variant_no_compatible_variant() {
    // maya_utils has 2 variants: maya-2023 and maya-2024
    // But neither maya-2023 nor maya-2024 are available.
    let packages = make_packages(vec![
        ("python", "3.9.0", vec![], vec![]),
        (
            "maya_utils",
            "1.0.0",
            vec!["python-3"],
            vec![vec!["maya-2023"], vec!["maya-2024"]],
        ),
    ]);
    let mut lp = LocalPackages::from_packages(packages);
    let result = solver_with_packages(vec!["maya_utils-1.0.0"], &mut lp);
    assert!(result.is_err(), "Should fail when no variant is satisfiable");
}

// ---------------------------------------------------------------------------
// Variant selector packages should NOT appear in solution output
// ---------------------------------------------------------------------------

#[test]
fn test_variant_selector_not_in_output() {
    let packages = make_packages(vec![
        ("maya", "2024.0.0", vec![], vec![]),
        ("maya", "2025.0.0", vec![], vec![]),
        (
            "tool",
            "1.0.0",
            vec![],
            vec![vec!["maya-2024"], vec!["maya-2025"]],
        ),
    ]);
    let mut lp = LocalPackages::from_packages(packages);
    let result = solver_with_packages(vec!["tool-1.0.0"], &mut lp).unwrap();

    // No entry should contain variant selector artifacts in the output
    for entry in &result {
        assert!(
            !entry.contains("__rer_internal_variant_selector__"),
            "Variant selector '{}' should not appear in output",
            entry
        );
        assert!(
            !entry.contains("__root__"),
            "Root package '{}' should not appear in output",
            entry
        );
    }
}

// ---------------------------------------------------------------------------
// Multiple versions of same package with different variants
// ---------------------------------------------------------------------------

#[test]
fn test_multi_version_with_variants() {
    // tool v1.0.0 has variants: [maya-2024], [maya-2025]
    // tool v2.0.0 has variants: [maya-2025], [maya-2026]
    // Only maya-2026 is available → solver should pick tool/2.0.0 variant 1
    let packages = make_packages(vec![
        ("maya", "2026.0.0", vec![], vec![]),
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
            vec![vec!["maya-2025"], vec!["maya-2026"]],
        ),
    ]);
    let mut lp = LocalPackages::from_packages(packages);
    let result = solver_with_packages(vec!["tool"], &mut lp).unwrap();
    let result = sorted(result);

    assert_eq!(
        result,
        sorted(vec![
            "tool/2.0.0/package.py".to_string(),
            "maya/2026.0.0/package.py".to_string(),
        ])
    );
}
