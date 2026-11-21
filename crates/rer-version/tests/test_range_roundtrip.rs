//! Dump random `VersionRange`s for the rez round-trip check.
//!
//! The `version_range` hint pyrer hands to a Python `load_family` callback is
//! `VersionRange::to_string()`, which rez must parse back to the same set.
//! This test builds ranges with the set operations the solver uses and writes
//! `(string, is_empty, [(probe, contained)])` records as JSON;
//! `scripts/check_range_roundtrip.py` checks them against rez's
//! `VersionRange`. `#[ignore]`d; run through that script.

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rer_version::{RerVersion, VersionRange};

const TOKENS: &[&str] = &["0", "1", "2", "3", "7", "10", "beta", "m1", "1a"];

fn version(r: &mut StdRng) -> String {
    let n = r.gen_range(1..=4);
    let mut s = String::new();
    for i in 0..n {
        if i > 0 {
            s.push(if r.gen_bool(0.3) { '-' } else { '.' });
        }
        s.push_str(TOKENS[r.gen_range(0..TOKENS.len())]);
    }
    s
}

fn atom(r: &mut StdRng) -> String {
    let (a, b) = (version(r), version(r));
    match r.gen_range(0..9) {
        0 => a,
        1 => format!("=={a}"),
        2 => format!("{a}+"),
        3 => format!(">{a}"),
        4 => format!(">={a}"),
        5 => format!("<{a}"),
        6 => format!("<={a}"),
        7 => {
            // `lo+<hi` needs lo < hi: rez rejects an empty bound ("Invalid
            // bound"), and so does rer's parse (it panics).
            let (va, vb) = (
                RerVersion::try_from(a.as_str()).unwrap(),
                RerVersion::try_from(b.as_str()).unwrap(),
            );
            match va.cmp(&vb) {
                std::cmp::Ordering::Less => format!("{a}+<{b}"),
                std::cmp::Ordering::Greater => format!("{b}+<{a}"),
                std::cmp::Ordering::Equal => format!("{a}+"),
            }
        }
        _ => format!("{a}|{b}"),
    }
}

fn range(r: &mut StdRng, depth: u32) -> VersionRange {
    if depth == 0 || r.gen_bool(0.3) {
        return VersionRange::parse(&atom(r));
    }
    let a = range(r, depth - 1);
    let b = range(r, depth - 1);
    match r.gen_range(0..4) {
        0 => a.intersection(&b).unwrap_or_else(VersionRange::empty),
        1 => a.union(&b),
        2 => a.inverse().unwrap_or_else(VersionRange::empty),
        _ => a.difference(&b).unwrap_or_else(VersionRange::empty),
    }
}

#[test]
#[ignore = "run via scripts/check_range_roundtrip.py"]
fn dump_ranges() {
    let out = std::env::var("RER_RANGE_DUMP").expect("RER_RANGE_DUMP");
    let n: usize = std::env::var("RER_RANGE_N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5000);
    let mut r = StdRng::seed_from_u64(0);
    let mut records = Vec::new();
    for _ in 0..n {
        let vr = range(&mut r, 3);
        let mut probes: Vec<String> = (0..12).map(|_| version(&mut r)).collect();
        probes.sort();
        probes.dedup();
        let members: Vec<String> = probes
            .iter()
            .map(|p| {
                let c = vr.contains(&RerVersion::try_from(p.as_str()).unwrap());
                format!("[{:?},{}]", p, c)
            })
            .collect();
        records.push(format!(
            "[{:?},{},[{}]]",
            vr.to_string(),
            vr.is_empty(),
            members.join(",")
        ));
    }
    std::fs::write(out, format!("[{}]", records.join(",\n"))).unwrap();
}
