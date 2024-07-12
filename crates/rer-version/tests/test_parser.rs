use rer_version::parser::VersionParsed;

#[test]
fn test_parser_exact_version() {
    let version = "1.2.3";
    let parsed_version = VersionParsed::parse_str(version);
    assert_eq!(parsed_version.exact_version, None);
    let version = "==1.2.3";
    let parsed_version = VersionParsed::parse_str(version);
    assert_eq!(parsed_version.exact_version, Some("==1.2.3"));
    assert_eq!(parsed_version.exact_version_group, Some("1.2.3"));
}

#[test]
fn test_parser_inclusive_bound() {
    let version = "1.2.3";
    let parsed_version = VersionParsed::parse_str(version);
    assert_eq!(parsed_version.inclusive_bound, None);
    let version = "1.2.3..2.0.0";
    let parsed_version = VersionParsed::parse_str(version);
    assert_eq!(parsed_version.inclusive_bound, Some("1.2.3..2.0.0"));
    assert_eq!(parsed_version.inclusive_lower_version, Some("1.2.3"));
    assert_eq!(parsed_version.inclusive_upper_version, Some("2.0.0"));
}
#[test]
fn test_parser_lower_bound() {
    let version = "1.2.3";
    let parsed_version = VersionParsed::parse_str(version);
    assert_eq!(parsed_version.lower_bound, None);
    let version = ">=1.2.3";
    let parsed_version = VersionParsed::parse_str(version);
    assert_eq!(parsed_version.lower_bound, Some(">=1.2.3"));
    assert_eq!(parsed_version.lower_bound_prefix, Some(">="));
    assert_eq!(parsed_version.lower_version, Some("1.2.3"));
    let version = ">=5.15.2.1";
    let parsed_version = VersionParsed::parse_str(version);
    assert_eq!(parsed_version.lower_bound, Some(">=5.15.2.1"));
    assert_eq!(parsed_version.lower_bound_prefix, Some(">="));
    assert_eq!(parsed_version.lower_version, Some("5.15.2.1"));
}
#[test]
fn test_ascending_range() {
    let version = "1.2.3+<2.0.0";
    let parsed_version = VersionParsed::parse_str(version);
    assert_eq!(parsed_version.range_asc, Some("1.2.3+<2.0.0"));
    assert_eq!(parsed_version.range_lower_asc, Some("1.2.3+"));
    assert_eq!(parsed_version.range_lower_asc_prefix, None);
    assert_eq!(parsed_version.range_lower_asc_version, Some("1.2.3"));
    assert_eq!(parsed_version.range_upper_asc, Some("<2.0.0"));
    assert_eq!(parsed_version.range_upper_asc_prefix, Some("<"));
    assert_eq!(parsed_version.range_upper_asc_version, Some("2.0.0"));
    let version = "5.15.2.1+<5.15.2.1.1";
    let parsed_version = VersionParsed::parse_str(version);
    assert_eq!(parsed_version.range_asc, Some("5.15.2.1+<5.15.2.1.1"));
    assert_eq!(parsed_version.range_lower_asc, Some("5.15.2.1+"));
}

#[test]
fn test_descending_range() {
    let version = "<=2.0.0,1.0.0+";
    let parsed_version = VersionParsed::parse_str(version);
    assert_eq!(parsed_version.range_desc, Some("<=2.0.0,1.0.0+"));
    assert_eq!(parsed_version.range_upper_desc, Some("<=2.0.0"));
    assert_eq!(parsed_version.range_upper_desc_prefix, Some("<="));
    assert_eq!(parsed_version.range_upper_desc_version, Some("2.0.0"));
    assert_eq!(parsed_version.range_lower_desc, Some("1.0.0+"));
    assert_eq!(parsed_version.range_lower_desc_version, Some("1.0.0"));
    assert_eq!(parsed_version.range_lower_desc_prefix, None);
}

#[test]
fn test_is_range() {
    let version = "1.2.3";
    let parsed_version = VersionParsed::parse_str(version);
    assert!(!parsed_version.is_range());
    let version = "1.2.3..2.0.0";
    let parsed_version = VersionParsed::parse_str(version);
    assert!(parsed_version.is_range());
    let version = "1.2.3+<2.0.0";
    let parsed_version = VersionParsed::parse_str(version);
    assert!(parsed_version.is_range());
    let version = "<=2.0.0,1.0.0+";
    let parsed_version = VersionParsed::parse_str(version);
    assert!(parsed_version.is_range());
}
