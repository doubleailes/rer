use std::fmt;

/// Identifies a package in the dependency resolver.
///
/// `PackageId` replaces raw `String` names throughout the pubgrub provider,
/// enabling clean distinction between regular packages, variant sub-packages,
/// and the virtual root of a resolve.
///
/// # Examples
///
/// ```
/// use rer_resolver::PackageId;
///
/// let base = PackageId::Base("foo".to_string());
/// assert_eq!(format!("{}", base), "foo");
///
/// let variant = PackageId::Variant("foo".to_string(), 1);
/// assert_eq!(format!("{}", variant), "foo[1]");
///
/// let root = PackageId::Root;
/// assert_eq!(format!("{}", root), "__root__");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PackageId {
    /// A regular (or multi-variant base) package, e.g. `"foo"`.
    Base(String),
    /// A concrete variant sub-package, e.g. `"foo"` variant `0`.
    Variant(String, usize),
    /// The virtual root package that represents the initial request.
    /// Replaces the `"init"` sentinel string.
    Root,
}

impl PackageId {
    /// Returns the package name for `Base` and `Variant`, or `None` for `Root`.
    pub fn name(&self) -> Option<&str> {
        match self {
            PackageId::Base(name) | PackageId::Variant(name, _) => Some(name),
            PackageId::Root => None,
        }
    }

    /// Returns `true` if this is a `Variant` package.
    pub fn is_variant(&self) -> bool {
        matches!(self, PackageId::Variant(..))
    }

    /// Returns `true` if this is the `Root` package.
    pub fn is_root(&self) -> bool {
        matches!(self, PackageId::Root)
    }
}

impl fmt::Display for PackageId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            PackageId::Base(name) => write!(f, "{}", name),
            PackageId::Variant(name, idx) => write!(f, "{}[{}]", name, idx),
            PackageId::Root => write!(f, "__root__"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_display_base() {
        let pkg = PackageId::Base("foo".to_string());
        assert_eq!(format!("{}", pkg), "foo");
    }

    #[test]
    fn test_display_variant() {
        let pkg = PackageId::Variant("foo".to_string(), 0);
        assert_eq!(format!("{}", pkg), "foo[0]");

        let pkg = PackageId::Variant("bar".to_string(), 3);
        assert_eq!(format!("{}", pkg), "bar[3]");
    }

    #[test]
    fn test_display_root() {
        let pkg = PackageId::Root;
        assert_eq!(format!("{}", pkg), "__root__");
    }

    #[test]
    fn test_name() {
        assert_eq!(PackageId::Base("foo".to_string()).name(), Some("foo"));
        assert_eq!(PackageId::Variant("bar".to_string(), 1).name(), Some("bar"));
        assert_eq!(PackageId::Root.name(), None);
    }

    #[test]
    fn test_is_variant() {
        assert!(!PackageId::Base("foo".to_string()).is_variant());
        assert!(PackageId::Variant("foo".to_string(), 0).is_variant());
        assert!(!PackageId::Root.is_variant());
    }

    #[test]
    fn test_is_root() {
        assert!(!PackageId::Base("foo".to_string()).is_root());
        assert!(!PackageId::Variant("foo".to_string(), 0).is_root());
        assert!(PackageId::Root.is_root());
    }

    #[test]
    fn test_equality() {
        assert_eq!(
            PackageId::Base("foo".to_string()),
            PackageId::Base("foo".to_string())
        );
        assert_ne!(
            PackageId::Base("foo".to_string()),
            PackageId::Base("bar".to_string())
        );
        assert_ne!(
            PackageId::Base("foo".to_string()),
            PackageId::Variant("foo".to_string(), 0)
        );
        assert_eq!(
            PackageId::Variant("foo".to_string(), 0),
            PackageId::Variant("foo".to_string(), 0)
        );
        assert_ne!(
            PackageId::Variant("foo".to_string(), 0),
            PackageId::Variant("foo".to_string(), 1)
        );
        assert_eq!(PackageId::Root, PackageId::Root);
    }

    #[test]
    fn test_hash_set() {
        let mut set = HashSet::new();
        set.insert(PackageId::Base("foo".to_string()));
        set.insert(PackageId::Variant("foo".to_string(), 0));
        set.insert(PackageId::Root);
        assert_eq!(set.len(), 3);

        // Duplicate insertion should not increase size
        set.insert(PackageId::Base("foo".to_string()));
        assert_eq!(set.len(), 3);
    }

    #[test]
    fn test_clone() {
        let pkg = PackageId::Variant("test".to_string(), 2);
        let cloned = pkg.clone();
        assert_eq!(pkg, cloned);
    }
}
