//! `_PackageScope` — one package family's evolving state during a solve.
//! Ported from `rez/src/rez/solver.py` (`_PackageScope`).
//!
//! A scope comes in three kinds:
//! - **normal** — backed by a [`PackageVariantSlice`]; the `package_request`
//!   is derived from the slice's version range.
//! - **conflict** — a `!foo` / `~foo` requirement; no slice, always "solved".
//!   Intersecting one with a positive range can *widen* it into a normal scope.
//! - **ephemeral** — a `.foo` requirement; no slice, just a range.

use super::context::SolverContext;
use super::requirement::Requirement;
use super::variant::{PackageVariant, PackageVariantSlice, Reduction, SliceIntersect, SliceReduce};
use rer_version::VersionRange;
use std::rc::Rc;

/// Why a [`PackageScope`] could not be constructed.
#[derive(Debug, Clone)]
pub enum ScopeError {
    /// The package family is not in the repository at all.
    FamilyNotFound(String),
    /// The family exists, but no version satisfies the requested range.
    PackageNotFound(Requirement),
}

impl std::fmt::Display for ScopeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScopeError::FamilyNotFound(name) => write!(f, "package family not found: {name}"),
            ScopeError::PackageNotFound(req) => write!(f, "package could not be found: {req}"),
        }
    }
}

/// Result of [`PackageScope::intersect`].
pub enum ScopeIntersect {
    /// The range removed nothing — the scope is unchanged.
    Unchanged,
    /// The range narrowed (or widened) the scope.
    Narrowed(PackageScope),
    /// The intersection emptied the scope — a conflict.
    Empty,
}

/// Result of [`PackageScope::reduce_by`].
pub enum ScopeReduce {
    /// Nothing was reduced — the scope is unchanged.
    Unchanged,
    /// Some variants were reduced away.
    Reduced(PackageScope),
    /// Every variant was reduced away — a total reduction.
    Empty,
}

/// One package family's possible solutions during a solve. Mirrors rez's
/// `_PackageScope`.
#[derive(Debug, Clone)]
pub struct PackageScope {
    ctx: Rc<SolverContext>,
    package_name: String,
    /// For conflict/ephemeral scopes this is the defining requirement; for a
    /// normal scope it is reconstructed from `variant_slice`'s range.
    package_request: Option<Requirement>,
    /// `Some` for a normal scope, `None` for conflict/ephemeral scopes.
    variant_slice: Option<PackageVariantSlice>,
    is_ephemeral: bool,
}

impl PackageScope {
    /// Build a scope from a requirement. Conflict (`!`/`~`) and ephemeral (`.`)
    /// requirements become slice-less scopes; everything else loads a variant
    /// slice (and fails if the family/version cannot be found).
    pub fn new(package_request: Requirement, ctx: &Rc<SolverContext>) -> Result<Self, ScopeError> {
        let package_name = package_request.name().to_string();
        let is_ephemeral = package_name.starts_with('.');

        if package_request.is_conflict() || is_ephemeral {
            // These cases do not contain variants.
            return Ok(PackageScope {
                ctx: Rc::clone(ctx),
                package_name,
                package_request: Some(package_request),
                variant_slice: None,
                is_ephemeral,
            });
        }

        // Normal scope: load the variant slice for the requested range.
        let range = package_request
            .range()
            .expect("a non-conflict requirement always has a range");
        match ctx.get_variant_slice(&package_name, range) {
            Some(slice) => {
                let mut scope = PackageScope {
                    ctx: Rc::clone(ctx),
                    package_name,
                    package_request: None,
                    variant_slice: Some(slice),
                    is_ephemeral: false,
                };
                scope.update();
                Ok(scope)
            }
            None => {
                if ctx.family_missing(&package_name) {
                    Err(ScopeError::FamilyNotFound(package_name))
                } else {
                    Err(ScopeError::PackageNotFound(Requirement::construct(
                        package_name,
                        package_request.range().cloned(),
                    )))
                }
            }
        }
    }

    /// The package family name.
    pub fn package_name(&self) -> &str {
        &self.package_name
    }

    /// The defining/derived requirement for this scope.
    pub fn package_request(&self) -> Option<&Requirement> {
        self.package_request.as_ref()
    }

    /// True if this is a conflict (`!foo` / `~foo`) scope.
    pub fn is_conflict(&self) -> bool {
        self.package_request
            .as_ref()
            .is_some_and(Requirement::is_conflict)
    }

    /// True if this is an ephemeral (`.foo`) scope.
    pub fn is_ephemeral(&self) -> bool {
        self.is_ephemeral
    }

    /// Reconstruct `package_request` from the current variant slice's range
    /// (rez's `_PackageScope._update`).
    fn update(&mut self) {
        if let Some(slice) = &self.variant_slice {
            self.package_request = Some(Requirement::construct(
                self.package_name.clone(),
                Some(slice.range()),
            ));
        }
    }

    /// rez's `_PackageScope._copy`: a shallow copy with a new slice, then
    /// `_update` to refresh the derived `package_request`.
    fn copy_with_slice(&self, new_slice: PackageVariantSlice) -> PackageScope {
        let mut scope = PackageScope {
            ctx: Rc::clone(&self.ctx),
            package_name: self.package_name.clone(),
            package_request: None,
            variant_slice: Some(new_slice),
            is_ephemeral: self.is_ephemeral,
        };
        scope.update();
        scope
    }

    /// Intersect this scope with a version range.
    pub fn intersect(&self, range: &VersionRange) -> ScopeIntersect {
        if self.is_ephemeral {
            return self.intersect_ephemeral(range);
        }

        if self.is_conflict() {
            // A conflict scope intersected with a positive range either widens
            // into a real (normal) scope, or conflicts away to nothing.
            let req = self.package_request.as_ref().unwrap();
            let new_slice = match req.range() {
                // `~foo` — no constraint; just load the slice for `range`.
                None => self.ctx.get_variant_slice(&self.package_name, range),
                // `!foo[-X]` — load the slice for `range` minus the forbidden part.
                Some(req_range) => match range.difference(req_range) {
                    Some(new_range) => self.ctx.get_variant_slice(&self.package_name, &new_range),
                    None => None,
                },
            };
            return match new_slice {
                None => ScopeIntersect::Empty,
                Some(slice) => ScopeIntersect::Narrowed(self.copy_with_slice(slice)),
            };
        }

        // Normal scope: intersect the variant slice.
        match self.variant_slice.as_ref().unwrap().intersect(range) {
            SliceIntersect::Empty => ScopeIntersect::Empty,
            SliceIntersect::Unchanged => ScopeIntersect::Unchanged,
            SliceIntersect::Narrowed(slice) => {
                ScopeIntersect::Narrowed(self.copy_with_slice(slice))
            }
        }
    }

    /// Ephemeral scopes are just a range intersection (`-` for conflict, `&`
    /// otherwise).
    fn intersect_ephemeral(&self, range: &VersionRange) -> ScopeIntersect {
        let req = self.package_request.as_ref().unwrap();
        let req_range = req
            .range()
            .expect("ephemeral scope requirement has a range");
        let intersect_range = if self.is_conflict() {
            range.difference(req_range)
        } else {
            range.intersection(req_range)
        };
        match intersect_range {
            None => ScopeIntersect::Empty,
            Some(ref ir) if ir == req_range => ScopeIntersect::Unchanged,
            Some(ir) => {
                let mut scope = self.clone();
                scope.package_request =
                    Some(Requirement::construct(self.package_name.clone(), Some(ir)));
                ScopeIntersect::Narrowed(scope)
            }
        }
    }

    /// Reduce this scope by a package request — remove variants whose
    /// dependencies conflict with it. Conflict/ephemeral scopes have no
    /// variants and are returned unchanged.
    pub fn reduce_by(&self, package_request: &Requirement) -> (ScopeReduce, Vec<Reduction>) {
        if self.is_conflict() || self.is_ephemeral {
            return (ScopeReduce::Unchanged, Vec::new());
        }
        let (result, reductions) = self
            .variant_slice
            .as_ref()
            .unwrap()
            .reduce_by(package_request);
        match result {
            SliceReduce::Empty => (ScopeReduce::Empty, reductions),
            SliceReduce::Reduced(slice) => (
                ScopeReduce::Reduced(self.copy_with_slice(slice)),
                reductions,
            ),
            SliceReduce::Unchanged => (ScopeReduce::Unchanged, Vec::new()),
        }
    }

    /// Extract a dependency common to every variant in this scope. Returns the
    /// new scope and the extracted requirement, or `None` if nothing can be
    /// extracted (including for conflict/ephemeral scopes).
    ///
    /// Note: unlike [`Self::copy_with_slice`], extraction does *not* refresh
    /// `package_request` — only `extracted_fams` changed, not the version range.
    pub fn extract(&self) -> Option<(PackageScope, Requirement)> {
        if self.is_conflict() || self.is_ephemeral {
            return None;
        }
        let (new_slice, package_request) = self.variant_slice.as_ref().unwrap().extract()?;
        let mut scope = self.clone();
        scope.variant_slice = Some(new_slice);
        Some((scope, package_request))
    }

    /// Split the scope into a preferred half (guaranteed to have a common
    /// dependency) and the remainder. `None` if the scope cannot be split
    /// (conflict, ephemeral, or only one variant left).
    pub fn split(&mut self) -> Option<(PackageScope, PackageScope)> {
        if self.is_conflict() || self.is_ephemeral {
            return None;
        }
        let slice = self.variant_slice.as_mut().unwrap();
        if slice.len() == 1 {
            return None;
        }
        let (slice_a, slice_b) = slice.split();
        Some((self.copy_with_slice(slice_a), self.copy_with_slice(slice_b)))
    }

    /// True if this scope is fully resolved: a conflict/ephemeral scope (always
    /// solved), or a normal scope down to a single, non-extractable variant.
    pub fn is_solved(&self) -> bool {
        if self.is_conflict() || self.is_ephemeral {
            return true;
        }
        let slice = self.variant_slice.as_ref().unwrap();
        slice.len() == 1 && !slice.extractable()
    }

    /// The single solved variant, if this scope is a resolved normal scope.
    pub fn get_solved_variant(&mut self) -> Option<Rc<PackageVariant>> {
        match &mut self.variant_slice {
            Some(slice) if slice.len() == 1 && !slice.extractable() => Some(slice.first_variant()),
            _ => None,
        }
    }

    /// The defining requirement, if this scope is a (non-conflict) ephemeral.
    pub fn get_solved_ephemeral(&self) -> Option<&Requirement> {
        if self.is_ephemeral && !self.is_conflict() {
            self.package_request.as_ref()
        } else {
            None
        }
    }
}

impl std::fmt::Display for PackageScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.variant_slice {
            Some(slice) => write!(f, "{slice}"),
            None => match &self.package_request {
                Some(req) => write!(f, "{req}"),
                None => write!(f, "{}", self.package_name),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::context::{PackageRepo, SolverContext};
    use super::super::requirement::{Requirement, RequirementList};
    use super::*;
    use crate::PackageData;

    fn pkg(requires: &[&str], variants: &[&[&str]]) -> PackageData {
        PackageData {
            requires: requires.iter().map(|s| s.to_string()).collect(),
            variants: variants
                .iter()
                .map(|v| v.iter().map(|s| s.to_string()).collect())
                .collect(),
        }
    }

    fn repo(entries: Vec<(&str, Vec<(&str, PackageData)>)>) -> PackageRepo {
        entries
            .into_iter()
            .map(|(name, versions)| {
                (
                    name.to_string(),
                    versions
                        .into_iter()
                        .map(|(v, d)| (v.to_string(), d))
                        .collect(),
                )
            })
            .collect()
    }

    fn ctx_with(repo: PackageRepo, requests: &[&str]) -> Rc<SolverContext> {
        let request_list =
            RequirementList::new(requests.iter().map(|s| Requirement::parse(s)).collect());
        Rc::new(SolverContext::new(repo, request_list))
    }

    #[test]
    fn test_normal_scope_construction_and_request() {
        let r = repo(vec![(
            "foo",
            vec![("1.0", pkg(&[], &[])), ("2.0", pkg(&[], &[]))],
        )]);
        let ctx = ctx_with(r, &["foo"]);
        let scope = PackageScope::new(Requirement::parse("foo"), &ctx).unwrap();
        assert!(!scope.is_conflict());
        assert!(!scope.is_ephemeral());
        // package_request is derived from the slice range, spanning 1.0 and 2.0
        assert!(!scope.is_solved()); // two variants
    }

    #[test]
    fn test_conflict_scope_is_always_solved() {
        let r = repo(vec![("foo", vec![("1.0", pkg(&[], &[]))])]);
        let ctx = ctx_with(r, &["!bar"]);
        let scope = PackageScope::new(Requirement::parse("!bar"), &ctx).unwrap();
        assert!(scope.is_conflict());
        assert!(scope.is_solved());
        // reduce/extract/split are no-ops on a conflict scope
        assert!(matches!(
            scope.reduce_by(&Requirement::parse("foo-1")).0,
            ScopeReduce::Unchanged
        ));
        assert!(scope.extract().is_none());
        assert!(scope.clone().split().is_none());
    }

    #[test]
    fn test_missing_family_and_package() {
        let r = repo(vec![("foo", vec![("1.0", pkg(&[], &[]))])]);
        let ctx = ctx_with(r, &["foo"]);
        // family absent entirely
        match PackageScope::new(Requirement::parse("nope"), &ctx) {
            Err(ScopeError::FamilyNotFound(n)) => assert_eq!(n, "nope"),
            other => panic!("expected FamilyNotFound, got {other:?}"),
        }
        // family present, but no version in range
        match PackageScope::new(Requirement::parse("foo-9"), &ctx) {
            Err(ScopeError::PackageNotFound(_)) => {}
            other => panic!("expected PackageNotFound, got {other:?}"),
        }
    }

    #[test]
    fn test_intersect_narrows_and_empties() {
        let r = repo(vec![(
            "foo",
            vec![
                ("1.0", pkg(&[], &[])),
                ("2.0", pkg(&[], &[])),
                ("3.0", pkg(&[], &[])),
            ],
        )]);
        let ctx = ctx_with(r, &["foo"]);
        let scope = PackageScope::new(Requirement::parse("foo"), &ctx).unwrap();

        match scope.intersect(&VersionRange::parse("2+")) {
            ScopeIntersect::Narrowed(s) => assert!(!s.is_solved()),
            _ => panic!("expected narrowed"),
        }
        match scope.intersect(&VersionRange::parse("2")) {
            ScopeIntersect::Narrowed(s) => assert!(s.is_solved()), // down to foo-2.0
            _ => panic!("expected narrowed to a single version"),
        }
        match scope.intersect(&VersionRange::parse("9")) {
            ScopeIntersect::Empty => {}
            _ => panic!("expected empty"),
        }
        assert!(matches!(
            scope.intersect(&VersionRange::any()),
            ScopeIntersect::Unchanged
        ));
    }

    #[test]
    fn test_conflict_scope_widens_on_intersect() {
        // `!foo-1` intersected with a `foo` request widens into a real scope
        // containing only foo-2.0 (foo-1.0 is forbidden).
        let r = repo(vec![(
            "foo",
            vec![("1.0", pkg(&[], &[])), ("2.0", pkg(&[], &[]))],
        )]);
        let ctx = ctx_with(r, &["!foo-1"]);
        let scope = PackageScope::new(Requirement::parse("!foo-1"), &ctx).unwrap();
        assert!(scope.is_conflict());
        match scope.intersect(&VersionRange::any()) {
            ScopeIntersect::Narrowed(widened) => {
                assert!(
                    !widened.is_conflict(),
                    "widened scope is now a normal scope"
                );
                assert!(widened.is_solved(), "only foo-2.0 remains");
            }
            _ => panic!("expected the conflict scope to widen"),
        }
    }

    #[test]
    fn test_solved_variant_extraction() {
        let r = repo(vec![("foo", vec![("1.0", pkg(&["bar-1"], &[]))])]);
        let ctx = ctx_with(r, &["foo"]);
        let scope = PackageScope::new(Requirement::parse("foo"), &ctx).unwrap();
        // one version, but it has an extractable common dependency (bar)
        assert!(!scope.is_solved());
        let (mut extracted, req) = scope.extract().expect("bar extractable");
        assert_eq!(req.name(), "bar");
        // after extraction, the scope is solved (single, non-extractable variant)
        assert!(extracted.is_solved());
        let variant = extracted.get_solved_variant().unwrap();
        assert_eq!(variant.version().to_string(), "1.0");
    }
}
