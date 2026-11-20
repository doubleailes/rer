use rer_version::RerVersion;

#[test]
fn test_compare_version() {
    let v1: RerVersion = "1.0.0".try_into().unwrap();
    let v2: RerVersion = "1.1.0".try_into().unwrap();
    assert!(v1 < v2);
    assert!(v1 < v1.bump());
    let v1: RerVersion = "baker-1".try_into().unwrap();
    let v2: RerVersion = "baker-2".try_into().unwrap();
    assert!(v1 < v2);
    assert!(v1 < v1.bump());
    assert!(v1.bump() < v2.bump());
    let v1: RerVersion = "0".try_into().unwrap();
    let v2: RerVersion = "1".try_into().unwrap();
    assert!(v1 < v2);
    let v1: RerVersion = "a".try_into().unwrap();
    let v2: RerVersion = "A".try_into().unwrap();
    assert!(v1 < v2);
    let v1: RerVersion = "_5".try_into().unwrap();
    let v2: RerVersion = "2".try_into().unwrap();
    assert!(v1 < v2);
    let v1: RerVersion = "beta3".try_into().unwrap();
    let v2: RerVersion = "3beta".try_into().unwrap();
    assert!(v1 < v2);
    let v1: RerVersion = "1.0.0a".try_into().unwrap();
    let v2: RerVersion = "1.0.0A".try_into().unwrap();
    assert!(v1 < v2);
    let v1: RerVersion = "1.0.0".try_into().unwrap();
    let v2: RerVersion = "1.0.0_".try_into().unwrap();
    assert!(v1 < v2);
    let v1: RerVersion = "1.0.0_".try_into().unwrap();
    let v2: RerVersion = "1.0.0a".try_into().unwrap();
    assert!(v1 < v2);
    let v1: RerVersion = "4.8.6.m1".try_into().unwrap();
    let v2: RerVersion = "4.8.6.m3".try_into().unwrap();
    assert!(v1 < v2);
    let v1: RerVersion = "1.0.0a".try_into().unwrap();
    let v2: RerVersion = "1.0.0a".try_into().unwrap();
    assert!(v1 == v2);
}

// rez's `Version.__eq__` / `__lt__` compare tokens only; separators are
// cosmetic ("the version ``1.0.0`` is equivalent to ``1-0-0``").
#[test]
fn test_separators_do_not_affect_equality_or_order() {
    use rer_version::{RerVersion, VersionRange};
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let dash = RerVersion::try_from("7-1.1.0").unwrap();
    let dot = RerVersion::try_from("7.1.1.0").unwrap();
    assert_eq!(dash, dot);
    assert_eq!(dash.cmp(&dot), std::cmp::Ordering::Equal);
    let h = |v: &RerVersion| {
        let mut s = DefaultHasher::new();
        v.hash(&mut s);
        s.finish()
    };
    assert_eq!(h(&dash), h(&dot));
    // Display keeps the original separators.
    assert_eq!(dash.to_string(), "7-1.1.0");

    // Rez benchmark request #84: "7-1.1.0" minus "==7.1.1.0" must not
    // produce a segment whose bounds are equal versions.
    let r = VersionRange::parse("7-1.1.0")
        .intersection(&VersionRange::parse("==7.1.1.0").inverse().unwrap())
        .unwrap();
    assert!(!r.contains(&dot));
    assert!(r.contains(&RerVersion::try_from("7.1.1.0.vrayFix").unwrap()));
    assert!(!r.to_string().contains("+<7.1.1.0"), "{r}");
}
