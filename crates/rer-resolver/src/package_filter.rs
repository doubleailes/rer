use regex::Regex;
use rer_version::RerVersion;
use std::fmt;

/// Trait for filtering packages during dependency resolution.
///
/// Implementors return `Some(reason)` if a package should be excluded from the
/// resolve, or `None` if the package is allowed. This mirrors rez's
/// `PackageFilterList` with rules like `TimestampRule`, `RegexRule`, and `GlobRule`.
///
/// # Examples
///
/// ```
/// use rer_resolver::package_filter::{PackageFilter, RegexFilter};
/// use regex::Regex;
/// use rer_version::RerVersion;
///
/// let filter = RegexFilter {
///     family: Some(Regex::new("^test_").unwrap()),
///     version: None,
/// };
/// let v: RerVersion = "1.0.0".try_into().unwrap();
/// assert!(filter.excludes("test_foo", &v, None).is_some());
/// assert!(filter.excludes("prod_foo", &v, None).is_none());
/// ```
pub trait PackageFilter: Send + Sync {
    /// Returns `Some(reason)` if the package should be excluded, `None` otherwise.
    fn excludes(&self, name: &str, version: &RerVersion, timestamp: Option<u64>) -> Option<String>;
}

/// Excludes packages whose family name or version string matches a regex pattern.
///
/// If both `family` and `version` are `Some`, both must match for the package
/// to be excluded. If only one is `Some`, only that one needs to match.
///
/// # Examples
///
/// ```
/// use rer_resolver::package_filter::{PackageFilter, RegexFilter};
/// use regex::Regex;
/// use rer_version::RerVersion;
///
/// // Exclude all packages starting with "internal_"
/// let filter = RegexFilter {
///     family: Some(Regex::new("^internal_").unwrap()),
///     version: None,
/// };
/// let v: RerVersion = "1.0.0".try_into().unwrap();
/// assert!(filter.excludes("internal_tool", &v, None).is_some());
/// assert!(filter.excludes("public_tool", &v, None).is_none());
/// ```
pub struct RegexFilter {
    /// Regex to match against the package family name.
    pub family: Option<Regex>,
    /// Regex to match against the version string.
    pub version: Option<Regex>,
}

impl PackageFilter for RegexFilter {
    fn excludes(&self, name: &str, version: &RerVersion, _timestamp: Option<u64>) -> Option<String> {
        let version_str = version.to_string();
        let family_matches = self.family.as_ref().map_or(true, |re| re.is_match(name));
        let version_matches = self
            .version
            .as_ref()
            .map_or(true, |re| re.is_match(&version_str));

        // If both fields are None, nothing to match — don't exclude
        if self.family.is_none() && self.version.is_none() {
            return None;
        }

        if family_matches && version_matches {
            let mut reason = String::from("excluded by regex filter:");
            if let Some(ref re) = self.family {
                reason.push_str(&format!(" family=/{}/", re));
            }
            if let Some(ref re) = self.version {
                reason.push_str(&format!(" version=/{}/", re));
            }
            Some(reason)
        } else {
            None
        }
    }
}

/// Excludes packages whose family name matches a glob pattern.
///
/// Supports `*` (any sequence of characters) and `?` (any single character).
///
/// # Examples
///
/// ```
/// use rer_resolver::package_filter::{PackageFilter, GlobFilter};
/// use rer_version::RerVersion;
///
/// let filter = GlobFilter::new("test_*").unwrap();
/// let v: RerVersion = "1.0.0".try_into().unwrap();
/// assert!(filter.excludes("test_foo", &v, None).is_some());
/// assert!(filter.excludes("prod_foo", &v, None).is_none());
/// ```
pub struct GlobFilter {
    /// The original glob pattern string.
    pub family: String,
    /// Compiled regex derived from the glob pattern.
    compiled: Regex,
}

impl GlobFilter {
    /// Create a new `GlobFilter` from a glob pattern string.
    ///
    /// Returns `Err` if the pattern cannot be compiled into a valid regex.
    pub fn new(pattern: &str) -> Result<Self, regex::Error> {
        let regex_str = glob_to_regex(pattern);
        let compiled = Regex::new(&regex_str)?;
        Ok(GlobFilter {
            family: pattern.to_string(),
            compiled,
        })
    }
}

impl PackageFilter for GlobFilter {
    fn excludes(&self, name: &str, _version: &RerVersion, _timestamp: Option<u64>) -> Option<String> {
        if self.compiled.is_match(name) {
            Some(format!(
                "excluded by glob filter: family matches '{}'",
                self.family
            ))
        } else {
            None
        }
    }
}

/// Convert a glob pattern to an anchored regex string.
///
/// Supports `*` (match any characters) and `?` (match single character).
/// All other regex-special characters are escaped.
fn glob_to_regex(pattern: &str) -> String {
    let mut regex = String::from("^");
    for ch in pattern.chars() {
        match ch {
            '*' => regex.push_str(".*"),
            '?' => regex.push('.'),
            '.' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' | '\\' => {
                regex.push('\\');
                regex.push(ch);
            }
            _ => regex.push(ch),
        }
    }
    regex.push('$');
    regex
}

/// Stub filter for timestamp-based exclusion.
///
/// Requires package timestamp data which is not yet available. This filter
/// currently never excludes any package. A follow-up issue will add timestamp
/// support once `PackageData` includes timestamps.
///
/// # Examples
///
/// ```
/// use rer_resolver::package_filter::{PackageFilter, TimestampFilter};
/// use rer_version::RerVersion;
///
/// let filter = TimestampFilter { before: 1700000000 };
/// let v: RerVersion = "1.0.0".try_into().unwrap();
/// // Without timestamp data, nothing is excluded
/// assert!(filter.excludes("foo", &v, None).is_none());
/// // With a timestamp older than `before`, the package is excluded
/// assert!(filter.excludes("foo", &v, Some(1600000000)).is_some());
/// ```
pub struct TimestampFilter {
    /// Exclude packages released before this Unix timestamp.
    pub before: u64,
}

impl PackageFilter for TimestampFilter {
    fn excludes(&self, name: &str, version: &RerVersion, timestamp: Option<u64>) -> Option<String> {
        match timestamp {
            Some(ts) if ts < self.before => Some(format!(
                "excluded by timestamp filter: {}/{} timestamp {} < {}",
                name, version, ts, self.before
            )),
            _ => None,
        }
    }
}

/// Composes multiple `PackageFilter`s. A package is excluded if **any** filter
/// in the list excludes it (logical OR).
///
/// # Examples
///
/// ```
/// use rer_resolver::package_filter::{PackageFilter, FilterList, GlobFilter};
/// use rer_version::RerVersion;
///
/// let mut filters = FilterList::new();
/// filters.add(Box::new(GlobFilter::new("test_*").unwrap()));
/// let v: RerVersion = "1.0.0".try_into().unwrap();
/// assert!(filters.excludes("test_foo", &v, None).is_some());
/// assert!(filters.excludes("prod_foo", &v, None).is_none());
/// ```
pub struct FilterList(Vec<Box<dyn PackageFilter>>);

impl fmt::Debug for FilterList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("FilterList")
            .field(&format!("[{} filter(s)]", self.0.len()))
            .finish()
    }
}

impl FilterList {
    /// Create an empty `FilterList`.
    pub fn new() -> Self {
        FilterList(Vec::new())
    }

    /// Add a filter to the list.
    pub fn add(&mut self, filter: Box<dyn PackageFilter>) {
        self.0.push(filter);
    }

    /// Returns `true` if the list contains no filters.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Default for FilterList {
    fn default() -> Self {
        Self::new()
    }
}

impl PackageFilter for FilterList {
    fn excludes(&self, name: &str, version: &RerVersion, timestamp: Option<u64>) -> Option<String> {
        for filter in &self.0 {
            if let Some(reason) = filter.excludes(name, version, timestamp) {
                log::debug!("Package {}/{} excluded: {}", name, version, reason);
                return Some(reason);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_regex_filter_family_only() {
        let filter = RegexFilter {
            family: Some(Regex::new("^internal_").unwrap()),
            version: None,
        };
        let v: RerVersion = "1.0.0".try_into().unwrap();
        assert!(filter.excludes("internal_tool", &v, None).is_some());
        assert!(filter.excludes("public_tool", &v, None).is_none());
    }

    #[test]
    fn test_regex_filter_version_only() {
        let filter = RegexFilter {
            family: None,
            version: Some(Regex::new("^0\\.").unwrap()),
        };
        let v_pre: RerVersion = "0.9.0".try_into().unwrap();
        let v_stable: RerVersion = "1.0.0".try_into().unwrap();
        assert!(filter.excludes("foo", &v_pre, None).is_some());
        assert!(filter.excludes("foo", &v_stable, None).is_none());
    }

    #[test]
    fn test_regex_filter_both() {
        let filter = RegexFilter {
            family: Some(Regex::new("^foo$").unwrap()),
            version: Some(Regex::new("^1\\.0").unwrap()),
        };
        let v1: RerVersion = "1.0.0".try_into().unwrap();
        let v2: RerVersion = "2.0.0".try_into().unwrap();
        // Both match → excluded
        assert!(filter.excludes("foo", &v1, None).is_some());
        // Family matches, version doesn't → not excluded
        assert!(filter.excludes("foo", &v2, None).is_none());
        // Family doesn't match → not excluded
        assert!(filter.excludes("bar", &v1, None).is_none());
    }

    #[test]
    fn test_regex_filter_both_none() {
        let filter = RegexFilter {
            family: None,
            version: None,
        };
        let v: RerVersion = "1.0.0".try_into().unwrap();
        // Both None → never excludes
        assert!(filter.excludes("anything", &v, None).is_none());
    }

    #[test]
    fn test_glob_filter_star() {
        let filter = GlobFilter::new("test_*").unwrap();
        let v: RerVersion = "1.0.0".try_into().unwrap();
        assert!(filter.excludes("test_foo", &v, None).is_some());
        assert!(filter.excludes("test_", &v, None).is_some());
        assert!(filter.excludes("foo_test", &v, None).is_none());
    }

    #[test]
    fn test_glob_filter_question() {
        let filter = GlobFilter::new("fo?").unwrap();
        let v: RerVersion = "1.0.0".try_into().unwrap();
        assert!(filter.excludes("foo", &v, None).is_some());
        assert!(filter.excludes("fob", &v, None).is_some());
        assert!(filter.excludes("fooo", &v, None).is_none());
        assert!(filter.excludes("fo", &v, None).is_none());
    }

    #[test]
    fn test_glob_filter_exact() {
        let filter = GlobFilter::new("foo").unwrap();
        let v: RerVersion = "1.0.0".try_into().unwrap();
        assert!(filter.excludes("foo", &v, None).is_some());
        assert!(filter.excludes("foobar", &v, None).is_none());
    }

    #[test]
    fn test_glob_filter_special_chars() {
        let filter = GlobFilter::new("foo.bar").unwrap();
        let v: RerVersion = "1.0.0".try_into().unwrap();
        assert!(filter.excludes("foo.bar", &v, None).is_some());
        // Dot should not act as regex wildcard
        assert!(filter.excludes("fooXbar", &v, None).is_none());
    }

    #[test]
    fn test_timestamp_filter_no_timestamp() {
        let filter = TimestampFilter { before: 1700000000 };
        let v: RerVersion = "1.0.0".try_into().unwrap();
        // No timestamp data → not excluded
        assert!(filter.excludes("foo", &v, None).is_none());
    }

    #[test]
    fn test_timestamp_filter_old_package() {
        let filter = TimestampFilter { before: 1700000000 };
        let v: RerVersion = "1.0.0".try_into().unwrap();
        assert!(filter.excludes("foo", &v, Some(1600000000)).is_some());
    }

    #[test]
    fn test_timestamp_filter_new_package() {
        let filter = TimestampFilter { before: 1700000000 };
        let v: RerVersion = "1.0.0".try_into().unwrap();
        assert!(filter.excludes("foo", &v, Some(1800000000)).is_none());
    }

    #[test]
    fn test_filter_list_empty() {
        let filters = FilterList::new();
        let v: RerVersion = "1.0.0".try_into().unwrap();
        assert!(filters.excludes("foo", &v, None).is_none());
        assert!(filters.is_empty());
    }

    #[test]
    fn test_filter_list_single() {
        let mut filters = FilterList::new();
        filters.add(Box::new(GlobFilter::new("test_*").unwrap()));
        let v: RerVersion = "1.0.0".try_into().unwrap();
        assert!(filters.excludes("test_foo", &v, None).is_some());
        assert!(filters.excludes("prod_foo", &v, None).is_none());
    }

    #[test]
    fn test_filter_list_multiple_or_semantics() {
        let mut filters = FilterList::new();
        filters.add(Box::new(GlobFilter::new("test_*").unwrap()));
        filters.add(Box::new(RegexFilter {
            family: Some(Regex::new("^internal_").unwrap()),
            version: None,
        }));
        let v: RerVersion = "1.0.0".try_into().unwrap();
        // Matches first filter
        assert!(filters.excludes("test_foo", &v, None).is_some());
        // Matches second filter
        assert!(filters.excludes("internal_bar", &v, None).is_some());
        // Matches neither
        assert!(filters.excludes("public_tool", &v, None).is_none());
    }

    #[test]
    fn test_glob_to_regex_conversion() {
        assert_eq!(glob_to_regex("foo"), "^foo$");
        assert_eq!(glob_to_regex("foo*"), "^foo.*$");
        assert_eq!(glob_to_regex("f?o"), "^f.o$");
        assert_eq!(glob_to_regex("foo.bar"), "^foo\\.bar$");
        assert_eq!(glob_to_regex("foo[0]"), "^foo\\[0\\]$");
    }
}
