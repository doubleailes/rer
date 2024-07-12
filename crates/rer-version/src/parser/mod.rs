use crate::description::RerVersion;
use lazy_static::lazy_static;
use pubgrub::{range::Range, version::Version};
use regex::Regex;

fn get_regex() -> String {
    let version_group = r"([0-9a-zA-Z_]+(?:[.-][0-9a-zA-Z_]+)*)"; // Example version group regex pattern

    let version_range_regex = format!(
        concat!(
            // Match a version number (e.g. 1.0.0)
            r"^(?P<version>{version_group})$",
            "|",
            // Or match an exact version number (e.g. ==1.0.0)
            r"^(?P<exact_version>==(?P<exact_version_group>{version_group})?)$",
            "|",
            // Or match an inclusive bound (e.g. 1.0.0..2.0.0)
            r"^(?P<inclusive_bound>(?P<inclusive_lower_version>{version_group})?\.\.(?P<inclusive_upper_version>{version_group})?)$",
            "|",
            // Or match a lower bound (e.g. 1.0.0+)
            r"^(?P<lower_bound>(?P<lower_bound_prefix>>|>=)?(?P<lower_version>{version_group})?(?P<lower_bound_suffix>\+?))$",
            "|",
            // Or match an upper bound (e.g. <=1.0.0)
            r"^(?P<upper_bound>(?P<upper_bound_prefix><|<=)?(?P<upper_version>{version_group})?)$",
            "|",
            // Or match a range in ascending order (e.g. 1.0.0+<2.0.0)
            r"^(?P<range_asc>(?P<range_lower_asc>(?P<range_lower_asc_prefix>>|>=)?(?P<range_lower_asc_version>{version_group})?(?P<range_lower_asc_suffix>\+?))(?P<range_upper_asc>,?(?P<range_upper_asc_prefix><|<=)?(?P<range_upper_asc_version>{version_group})?))$",
            "|",
            // Or match a range in descending order (e.g. <=2.0.0,1.0.0+)
            r"^(?P<range_desc>(?P<range_upper_desc>(?P<range_upper_desc_prefix><=?)?(?P<range_upper_desc_version>{version_group})(\+?)?)(,(?P<range_lower_desc>(?P<range_lower_desc_prefix><|<=|>=?)?(?P<range_lower_desc_version>{version_group})(\+?)?))?)$"
        ),
        version_group = version_group
    );
    version_range_regex
}

lazy_static! {
    static ref RE_REZ_PACKAGE: Regex =
        regex::Regex::new(&get_regex()).expect("Can't compile regex");
}
#[derive(Default)]
pub struct VersionParsed<'a> {
    pub version: Option<&'a str>,
    pub exact_version: Option<&'a str>,
    pub exact_version_group: Option<&'a str>,
    pub inclusive_bound: Option<&'a str>,
    pub inclusive_lower_version: Option<&'a str>,
    pub inclusive_upper_version: Option<&'a str>,
    pub lower_bound: Option<&'a str>,
    pub lower_bound_prefix: Option<&'a str>,
    pub lower_version: Option<&'a str>,
    pub upper_bound: Option<&'a str>,
    pub upper_bound_prefix: Option<&'a str>,
    pub upper_version: Option<&'a str>,
    pub range_asc: Option<&'a str>,
    pub range_lower_asc: Option<&'a str>,
    pub range_lower_asc_prefix: Option<&'a str>,
    pub range_lower_asc_version: Option<&'a str>,
    pub range_upper_asc: Option<&'a str>,
    pub range_upper_asc_prefix: Option<&'a str>,
    pub range_upper_asc_version: Option<&'a str>,
    pub range_desc: Option<&'a str>,
    pub range_upper_desc: Option<&'a str>,
    pub range_upper_desc_prefix: Option<&'a str>,
    pub range_upper_desc_version: Option<&'a str>,
    pub range_lower_desc: Option<&'a str>,
    pub range_lower_desc_prefix: Option<&'a str>,
    pub range_lower_desc_version: Option<&'a str>,
}

fn get_named_capture<'a>(regex: &regex::Captures<'a>, name: &str) -> Option<&'a str> {
    regex.name(name).map(|m| m.as_str())
}

impl VersionParsed<'_> {
    // TODO: bench this function to compare to version in REZ which a list of if
    pub fn parse_str(input: &str) -> VersionParsed<'_> {
        if input.is_empty() {
            return VersionParsed::default();
        }
        let version_range_regex = match RE_REZ_PACKAGE.captures(input) {
            Some(captures) => captures,
            None => panic!("Invalid version range {}", input),
        };
        let version = get_named_capture(&version_range_regex, "version");
        let exact_version = get_named_capture(&version_range_regex, "exact_version");
        let exact_version_group = get_named_capture(&version_range_regex, "exact_version_group");
        let inclusive_bound = get_named_capture(&version_range_regex, "inclusive_bound");
        let inclusive_lower_version =
            get_named_capture(&version_range_regex, "inclusive_lower_version");
        let inclusive_upper_version =
            get_named_capture(&version_range_regex, "inclusive_upper_version");
        let lower_bound = get_named_capture(&version_range_regex, "lower_bound");
        let lower_bound_prefix = get_named_capture(&version_range_regex, "lower_bound_prefix");
        let lower_version = get_named_capture(&version_range_regex, "lower_version");
        let upper_bound = get_named_capture(&version_range_regex, "upper_bound");
        let upper_bound_prefix = get_named_capture(&version_range_regex, "upper_bound_prefix");
        let upper_version = get_named_capture(&version_range_regex, "upper_version");
        let range_asc = get_named_capture(&version_range_regex, "range_asc");
        let range_lower_asc = get_named_capture(&version_range_regex, "range_lower_asc");
        let range_lower_asc_prefix =
            get_named_capture(&version_range_regex, "range_lower_asc_prefix");
        let range_lower_asc_version =
            get_named_capture(&version_range_regex, "range_lower_asc_version");
        let range_upper_asc = get_named_capture(&version_range_regex, "range_upper_asc");
        let range_upper_asc_prefix =
            get_named_capture(&version_range_regex, "range_upper_asc_prefix");
        let range_upper_asc_version =
            get_named_capture(&version_range_regex, "range_upper_asc_version");
        let range_desc = get_named_capture(&version_range_regex, "range_desc");
        let range_upper_desc = get_named_capture(&version_range_regex, "range_upper_desc");
        let range_upper_desc_prefix =
            get_named_capture(&version_range_regex, "range_upper_desc_prefix");
        let range_upper_desc_version =
            get_named_capture(&version_range_regex, "range_upper_desc_version");
        let range_lower_desc = get_named_capture(&version_range_regex, "range_lower_desc");
        let range_lower_desc_prefix =
            get_named_capture(&version_range_regex, "range_lower_desc_prefix");
        let range_lower_desc_version =
            get_named_capture(&version_range_regex, "range_lower_desc_version");
        VersionParsed {
            version,
            exact_version,
            exact_version_group,
            inclusive_bound,
            inclusive_lower_version,
            inclusive_upper_version,
            lower_bound,
            lower_bound_prefix,
            lower_version,
            upper_bound,
            upper_bound_prefix,
            upper_version,
            range_asc,
            range_lower_asc,
            range_lower_asc_prefix,
            range_lower_asc_version,
            range_upper_asc,
            range_upper_asc_prefix,
            range_upper_asc_version,
            range_desc,
            range_upper_desc,
            range_upper_desc_prefix,
            range_upper_desc_version,
            range_lower_desc,
            range_lower_desc_prefix,
            range_lower_desc_version,
        }
    }
    pub fn is_range(&self) -> bool {
        self.inclusive_bound.is_some()
            || self.lower_bound.is_some()
            || self.upper_bound.is_some()
            || self.range_asc.is_some()
            || self.range_desc.is_some()
    }
}

pub fn parse_version_range(input_str: &str) -> Range<RerVersion> {
    if input_str.is_empty() {
        return Range::any();
    }
    let parsed = VersionParsed::parse_str(input_str);
    if !parsed.is_range() {
        if parsed.exact_version.is_some() {
            let v: RerVersion = parsed.exact_version_group.unwrap().try_into().unwrap();
            return Range::exact(v);
        }
        let v: RerVersion = parsed.version.unwrap().try_into().unwrap();
        return Range::between(v.clone(), v.bump());
    }
    if parsed.inclusive_bound.is_some() {
        let start: RerVersion = parsed.inclusive_lower_version.unwrap().try_into().unwrap();
        let end: RerVersion = parsed.inclusive_upper_version.unwrap().try_into().unwrap();
        let b = Range::between(start, end.clone());
        return b.union(&Range::exact(end));
    }
    if parsed.lower_bound.is_some() {
        let start: RerVersion = parsed.lower_version.unwrap().try_into().unwrap();
        return Range::higher_than(start);
    }
    if parsed.upper_bound.is_some() {
        let end: RerVersion = parsed.upper_version.unwrap().try_into().unwrap();
        if parsed.upper_bound_prefix == Some("<") {
            return Range::strictly_lower_than(end);
        }
        return Range::strictly_lower_than(end.clone()).union(&Range::exact(end));
    }
    if parsed.range_asc.is_some() {
        let start: RerVersion = parsed.range_lower_asc_version.unwrap().try_into().unwrap();
        let end: RerVersion = parsed.range_upper_asc_version.unwrap().try_into().unwrap();
        return Range::between(start, end);
    }
    if parsed.range_desc.is_some() {
        let end: RerVersion = parsed.range_upper_desc_version.unwrap().try_into().unwrap();
        let start: RerVersion = parsed.range_lower_desc_version.unwrap().try_into().unwrap();
        if parsed.range_upper_desc_prefix == Some("<") {
            return Range::between(start, end);
        }
        return Range::between(start, end.clone()).union(&Range::exact(end));
    }
    Range::any()
}

#[test]
fn test_parse_version_range() {
    let input_str = "";
    let a = parse_version_range(input_str);
    assert_eq!(a, Range::any());
    let input_str = "1";
    let a = parse_version_range(input_str);
    let v: RerVersion = input_str.try_into().unwrap();
    let r = Range::between(v.clone(), v.bump());
    assert_eq!(a, r);
    let v_sub_version: RerVersion = "1.0.0".try_into().unwrap();
    assert_eq!(a.contains(&v_sub_version), true);
    let input_str = "1.0.0";
    let a = parse_version_range(input_str);
    let v: RerVersion = input_str.try_into().unwrap();
    assert_eq!(a, Range::between(v.clone(), v.bump()));
    let input_str = "==1.0.0";
    let a = parse_version_range(input_str);
    let v: RerVersion = "1.0.0".try_into().unwrap();
    assert_eq!(a, Range::exact(v));
    let input_str = "1.0.0..2.0.0";
    let a = parse_version_range(input_str);
    let start: RerVersion = "1.0.0".try_into().unwrap();
    let end: RerVersion = "2.0.0_".try_into().unwrap();
    assert_eq!(a, Range::between(start, end));
    let input_str = "1.0.0+";
    let a = parse_version_range(input_str);
    let start: RerVersion = "1.0.0".try_into().unwrap();
    assert_eq!(a, Range::higher_than(start));
    let input_str = "<=1.0.0";
    let a = parse_version_range(input_str);
    let end: RerVersion = "1.0.0".try_into().unwrap();
    assert_eq!(
        a,
        Range::strictly_lower_than(end.clone()).union(&Range::exact(end))
    );
    let input_str = "<1.0.0";
    let a = parse_version_range(input_str);
    let end: RerVersion = "1.0.0".try_into().unwrap();
    assert_eq!(a, Range::strictly_lower_than(end));
    let input_str = "1.0.0+<2.0.0";
    let a = parse_version_range(input_str);
    let start: RerVersion = "1.0.0".try_into().unwrap();
    let end: RerVersion = "2.0.0".try_into().unwrap();
    assert_eq!(a, Range::between(start, end));
    let input_str = "<=2.0.0,1.0.0+";
    let a = parse_version_range(input_str);
    let end: RerVersion = "2.0.0".try_into().unwrap();
    let start: RerVersion = "1.0.0".try_into().unwrap();
    assert_eq!(
        a,
        Range::between(start, end.clone()).union(&Range::exact(end))
    );
    let input_str = "<2.0.0,1.0.0+";
    let a = parse_version_range(input_str);
    let end: RerVersion = "2.0.0".try_into().unwrap();
    let start: RerVersion = "1.0.0".try_into().unwrap();
    assert_eq!(a, Range::between(start, end));
}
