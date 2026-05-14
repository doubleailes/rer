//! [`PackageData`] — the in-memory description of a single package version.
//!
//! This is the unit of the package repository the solver works on
//! (`family -> version -> PackageData`); the host loads it and hands it over,
//! rer does not read the filesystem itself.

/// Data for a single package version: base requirements plus variant-specific
/// dependencies.
///
/// For a package like:
/// ```text
/// name = "maya_utils"
/// version = "1.0.0"
/// requires = ["python-3"]
/// variants = [["maya-2024"], ["maya-2025"]]
/// ```
///
/// `requires` would be `["python-3"]` and `variants` would be
/// `[["maya-2024"], ["maya-2025"]]`.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct PackageData {
    /// Base requirements that apply to all variants.
    #[serde(default)]
    pub requires: Vec<String>,
    /// Each inner `Vec` is a list of dependency strings for one variant.
    /// Empty if the package defines no variants.
    #[serde(default)]
    pub variants: Vec<Vec<String>>,
}

impl PackageData {
    /// Returns true if the package defines more than one variant.
    pub fn is_multi_variant(&self) -> bool {
        self.variants.len() > 1
    }

    /// The combined requirements for a single-variant or no-variant package.
    /// For multi-variant packages, returns only the base `requires`.
    pub fn combined_requirements(&self) -> Vec<String> {
        let mut deps = self.requires.clone();
        if self.variants.len() == 1 {
            deps.extend(self.variants[0].clone());
        }
        deps
    }
}
