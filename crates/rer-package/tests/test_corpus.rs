//! Run `parse_static_package_py` against a real rez repo and report
//! the accept rate. Mirrors the Python survey at
//! `scripts/survey_package_py.py` — running both against the same
//! corpus should produce matching `fast-parseable` counts.
//!
//! `#[ignore]`d: this needs an external rez package store. Run with:
//!
//! ```text
//! RER_CORPUS_PATH=/thierry/rez/pkg \
//!   cargo test --release -p rer-package --test test_corpus -- --ignored
//! ```
//!
//! Set `RER_CORPUS_REQUIRE` to a percentage to gate on a minimum:
//!
//! ```text
//! RER_CORPUS_REQUIRE=90 RER_CORPUS_PATH=/thierry/rez/pkg \
//!   cargo test --release -p rer-package --test test_corpus -- --ignored
//! ```
//!
//! Without `RER_CORPUS_PATH`, the test prints a skip message and
//! passes (so CI on a checkout without the mount doesn't fail).

use std::path::Path;

#[test]
#[ignore = "external rez corpus required; see RER_CORPUS_PATH"]
fn corpus_accept_rate() {
    let Some(root) = std::env::var_os("RER_CORPUS_PATH") else {
        eprintln!("RER_CORPUS_PATH not set — skipping corpus check.");
        return;
    };
    let root: &Path = root.as_ref();
    if !root.is_dir() {
        eprintln!("RER_CORPUS_PATH is not a directory: {}", root.display());
        return;
    }

    let mut total = 0usize;
    let mut accepted = 0usize;
    let mut read_errors = 0usize;

    walk_package_pys(root, &mut |path: &Path| {
        total += 1;
        match std::fs::read_to_string(path) {
            Ok(src) => {
                if rer_package::parse_static_package_py(&src).is_some() {
                    accepted += 1;
                }
            }
            Err(_) => read_errors += 1,
        }
    });

    let pct = if total > 0 {
        accepted as f64 / total as f64 * 100.0
    } else {
        0.0
    };
    println!(
        "rer-package on {}: total {total}, accepted {accepted} ({pct:.1}%), read errors {read_errors}",
        root.display()
    );

    if let Ok(min) = std::env::var("RER_CORPUS_REQUIRE") {
        let min: f64 = min.parse().expect("RER_CORPUS_REQUIRE must be a number");
        assert!(
            pct >= min,
            "accept rate {pct:.1}% < required {min:.1}%"
        );
    } else {
        // No gate set — just don't fail the test.
        assert!(total > 0, "no package.py files found under {}", root.display());
    }
}

/// Recursively yield every `package.py` under `root`, skipping
/// dot-directories, leading-underscore directories, and rez variant
/// subdirs (40-char hex hashes). Matches the Python survey's traversal
/// so the two tools see the same set of files.
fn walk_package_pys(root: &Path, f: &mut dyn FnMut(&Path)) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name.starts_with('_') {
            continue;
        }
        if name.len() == 40 && name.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        let ty = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if ty.is_dir() {
            walk_package_pys(&path, f);
        } else if ty.is_file() && name == "package.py" {
            f(&path);
        }
    }
}
