/// Differential test harness: rer vs expected rez solver results.
///
/// Loads test cases from `tests/differential_cases.json`, each containing a
/// package repository subset, request strings, and expected solve outcomes.
///
/// # Known-Acceptable Divergences
///
/// The pubgrub-based solver may produce *different but equally valid* solutions
/// compared to rez's Python solver. Both are correct if:
///
/// 1. All resolved versions satisfy every requirement in the request.
/// 2. All transitive dependencies are satisfied.
/// 3. The status (solved/failed) matches.
///
/// When `expected_resolved` is `null` in a test case, only the status is
/// checked. When it is set, the exact package set is verified — these are
/// cases where only one valid solution exists (e.g., leaf packages).
///
/// Cases with `expected_status = "solved_or_failed"` are baseline cases from
/// real package data (`rez_lib_caviar.json`). They accept either outcome and
/// serve as regression anchors — if a future change flips the status, the test
/// will catch it once the baseline is locked in.
use rer_resolver::{solver_with_packages, LocalPackages, PackageData};
use serde::Deserialize;
use std::collections::HashMap;

/// Top-level test fixture file format.
#[derive(Deserialize)]
struct TestFixture {
    cases: Vec<TestCase>,
}

/// A single differential test case.
#[derive(Deserialize)]
struct TestCase {
    id: String,
    requests: Vec<String>,
    /// Package name → version → dependency list.
    packages: HashMap<String, HashMap<String, Vec<String>>>,
    expected_status: String,
    /// If set, the exact set of `[name, version]` pairs expected.
    expected_resolved: Option<Vec<Vec<String>>>,
    #[allow(dead_code)]
    notes: String,
    category: String,
    #[allow(dead_code)]
    description: String,
}

/// Convert the JSON package map to `LocalPackages` using `from_packages()`.
fn build_local_packages(
    packages: &HashMap<String, HashMap<String, Vec<String>>>,
) -> LocalPackages {
    let mut pkg_data: HashMap<String, HashMap<String, PackageData>> = HashMap::new();
    for (pkg_name, versions) in packages {
        let mut version_map: HashMap<String, PackageData> = HashMap::new();
        for (version, deps) in versions {
            version_map.insert(
                version.clone(),
                PackageData {
                    requires: deps.clone(),
                    variants: Vec::new(),
                },
            );
        }
        pkg_data.insert(pkg_name.clone(), version_map);
    }
    LocalPackages::from_packages(pkg_data)
}

/// Parse solver output "name/version/package.py" into (name, version).
fn parse_resolved(entries: &[String]) -> Vec<(String, String)> {
    let mut result: Vec<(String, String)> = entries
        .iter()
        .filter_map(|entry| {
            let parts: Vec<&str> = entry.splitn(3, '/').collect();
            if parts.len() >= 2 {
                Some((parts[0].to_string(), parts[1].to_string()))
            } else {
                None
            }
        })
        .collect();
    result.sort();
    result
}

fn load_cases() -> Vec<TestCase> {
    let fixture_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/differential_cases.json");
    let data = std::fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {}", fixture_path.display(), e));
    let fixture: TestFixture =
        serde_json::from_str(&data).expect("Failed to parse differential_cases.json");
    fixture.cases
}

// ---------------------------------------------------------------------------
// The main differential test: runs every case from the fixture file
// ---------------------------------------------------------------------------

#[test]
fn test_differential_all_cases() {
    let cases = load_cases();
    assert!(
        cases.len() >= 50,
        "Expected at least 50 test cases, got {}",
        cases.len()
    );

    let mut passed = 0;
    let mut failed_tests: Vec<String> = Vec::new();

    for case in &cases {
        let req_strs: Vec<&str> = case.requests.iter().map(|s| s.as_str()).collect();
        let mut lp = build_local_packages(&case.packages);
        let result = solver_with_packages(req_strs, &mut lp);

        let actual_status = if result.is_ok() { "solved" } else { "failed" };

        // "solved_or_failed" accepts either outcome (baseline/real-data cases)
        if case.expected_status != "solved_or_failed" && actual_status != case.expected_status {
            failed_tests.push(format!(
                "[{}] status mismatch: expected '{}', got '{}'",
                case.id, case.expected_status, actual_status
            ));
            continue;
        }

        // If solved and expected_resolved is specified, check exact match
        if actual_status == "solved" {
            if let Some(ref expected) = case.expected_resolved {
                let resolved = parse_resolved(result.as_ref().unwrap());
                let mut expected_pairs: Vec<(String, String)> = expected
                    .iter()
                    .map(|pair| (pair[0].clone(), pair[1].clone()))
                    .collect();
                expected_pairs.sort();

                if resolved != expected_pairs {
                    failed_tests.push(format!(
                        "[{}] resolved mismatch: expected {:?}, got {:?}",
                        case.id, expected_pairs, resolved
                    ));
                    continue;
                }
            }
        }

        passed += 1;
    }

    if !failed_tests.is_empty() {
        panic!(
            "{} of {} differential tests failed:\n{}",
            failed_tests.len(),
            cases.len(),
            failed_tests.join("\n")
        );
    }

    assert_eq!(passed, cases.len(), "Not all cases passed");
}

// ---------------------------------------------------------------------------
// Verify that each solved result contains only packages from the repository
// ---------------------------------------------------------------------------

#[test]
fn test_differential_resolved_packages_exist_in_repo() {
    let cases = load_cases();

    for case in &cases {
        let req_strs: Vec<&str> = case.requests.iter().map(|s| s.as_str()).collect();
        let mut lp = build_local_packages(&case.packages);

        if let Ok(solution) = solver_with_packages(req_strs, &mut lp) {
            let resolved = parse_resolved(&solution);
            for (name, version) in &resolved {
                assert!(
                    case.packages.contains_key(name),
                    "[{}] resolved package '{}' not in repository",
                    case.id,
                    name
                );
                assert!(
                    case.packages[name].contains_key(version),
                    "[{}] resolved version '{}/{}' not in repository",
                    case.id,
                    name,
                    version
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Verify minimum case count per category
// ---------------------------------------------------------------------------

#[test]
fn test_differential_case_coverage() {
    let cases = load_cases();

    let leaf_count = cases.iter().filter(|c| c.category == "leaf").count();
    let missing_count = cases.iter().filter(|c| c.category == "missing").count();
    let multi_count = cases
        .iter()
        .filter(|c| c.category.starts_with("multi"))
        .count();
    let exact_count = cases
        .iter()
        .filter(|c| c.category == "exact_version")
        .count();
    let conflict_count = cases.iter().filter(|c| c.category == "conflict").count();
    let real_count = cases.iter().filter(|c| c.category == "real_deps").count();

    assert!(leaf_count >= 10, "Need at least 10 leaf cases");
    assert!(missing_count >= 3, "Need at least 3 missing-package cases");
    assert!(
        multi_count >= 5,
        "Need at least 5 multi-package request cases"
    );
    assert!(
        exact_count >= 3,
        "Need at least 3 exact-version cases"
    );
    assert!(
        conflict_count >= 1,
        "Need at least 1 conflict case"
    );
    assert!(
        real_count >= 10,
        "Need at least 10 real-world dependency cases"
    );
}
