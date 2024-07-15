use pubgrub::version::Version;
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
    let v1: RerVersion = "1.0.0a".try_into().unwrap();
    let v2: RerVersion = "1.0.0a".try_into().unwrap();
    assert!(v1 == v2);
}
