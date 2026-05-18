//! `Solver` — the top-level driver: a phase stack, depth-first search with
//! implicit backtracking. Ported from `rez/src/rez/solver.py` (`Solver`).
//!
//! Backtracking is implicit in the stack: when a phase fails it is pushed; the
//! next step pops and archives it, then resumes the alternative phase beneath
//! (the "without selection" half produced by an earlier `split`).

use super::context::{SharedVariantCache, SolverContext, VariantSelectMode};
use super::failure::{FailureReason, SolverStatus};
use super::phase::ResolvePhase;
use super::requirement::{Requirement, RequirementList};
use super::scope::ScopeError;
use super::variant::PackageVariant;
use crate::rez_solver::context::PackageRepo;
use std::rc::Rc;

/// The top-level package solver. Mirrors rez's `Solver`.
pub struct Solver {
    ctx: Rc<SolverContext>,
    phase_stack: Vec<ResolvePhase>,
    failed_phase_list: Vec<ResolvePhase>,
    solve_count: usize,
}

impl Solver {
    /// Create a solver for `package_requests` against `repo`.
    ///
    /// The repository is shared via `Rc`, so resolving many requests against
    /// the same repo never copies it. Fails only if a top-level request names
    /// a package family or version absent from the repository (rez raises too).
    ///
    /// A fresh per-solver variant cache is built. For repeated solves against
    /// the same repository, prefer [`Self::new_with_cache`] — it skips
    /// re-parsing every variant's requires on each call (about 22 % of solve
    /// time on the rez benchmark).
    pub fn new(
        package_requests: Vec<Requirement>,
        repo: Rc<PackageRepo>,
    ) -> Result<Self, ScopeError> {
        Self::new_with_cache(package_requests, repo, super::context::make_shared_cache())
    }

    /// Create a solver sharing the given variant cache.
    ///
    /// The caller is responsible for ensuring that `cache` was built from the
    /// same `repo`. A cache memoises both "family present?" and the parsed
    /// `PackageVariantList` for each family — both of which are wrong against
    /// a different repository. Uses the default variant-select mode
    /// (`version_priority`); for `intersection_priority`, use
    /// [`Self::new_with_options`].
    pub fn new_with_cache(
        package_requests: Vec<Requirement>,
        repo: Rc<PackageRepo>,
        cache: SharedVariantCache,
    ) -> Result<Self, ScopeError> {
        Self::new_with_options(package_requests, repo, cache, VariantSelectMode::default())
    }

    /// Create a solver with full control over both the shared cache and the
    /// variant-select mode. Use this when you need `intersection_priority`
    /// or when wiring rez's `config.variant_select_mode` through.
    pub fn new_with_options(
        package_requests: Vec<Requirement>,
        repo: Rc<PackageRepo>,
        cache: SharedVariantCache,
        variant_select_mode: VariantSelectMode,
    ) -> Result<Self, ScopeError> {
        let request_list = RequirementList::new(package_requests);

        let build_ctx =
            |repo: Rc<PackageRepo>, request_list: RequirementList| -> Rc<SolverContext> {
                Rc::new(
                    SolverContext::new_with_cache(repo, request_list, Rc::clone(&cache))
                        .with_variant_select_mode(variant_select_mode),
                )
            };

        // A conflicting request fails immediately, with no scopes.
        if let Some((req1, req2)) = request_list.conflict() {
            let failure =
                FailureReason::DependencyConflicts(vec![super::failure::DependencyConflict {
                    dependency: req1.clone(),
                    conflicting_request: req2.clone(),
                }]);
            let ctx = build_ctx(repo, request_list);
            let phase = ResolvePhase::failed(&ctx, failure);
            return Ok(Solver {
                ctx,
                phase_stack: vec![phase],
                failed_phase_list: Vec::new(),
                solve_count: 0,
            });
        }

        let ctx = build_ctx(repo, request_list);
        let phase = ResolvePhase::new(&ctx)?;
        Ok(Solver {
            ctx,
            phase_stack: vec![phase],
            failed_phase_list: Vec::new(),
            solve_count: 0,
        })
    }

    /// The current solver status. Mirrors rez's `Solver.status`.
    pub fn status(&self) -> SolverStatus {
        if self.ctx.request_list.conflict().is_some() {
            return SolverStatus::Failed;
        }
        let st = self.phase_stack.last().unwrap().status;
        if st == SolverStatus::Cyclic {
            SolverStatus::Failed
        } else if self.phase_stack.len() > 1 {
            if st == SolverStatus::Solved {
                SolverStatus::Solved
            } else {
                SolverStatus::Unsolved
            }
        } else if matches!(st, SolverStatus::Pending | SolverStatus::Exhausted) {
            SolverStatus::Unsolved
        } else {
            st
        }
    }

    /// Number of solve steps executed.
    pub fn num_solves(&self) -> usize {
        self.solve_count
    }

    /// Number of failed solve steps.
    pub fn num_fails(&self) -> usize {
        let mut n = self.failed_phase_list.len();
        if matches!(
            self.phase_stack.last().unwrap().status,
            SolverStatus::Failed | SolverStatus::Cyclic
        ) {
            n += 1;
        }
        n
    }

    /// Run the solve to completion. Mirrors rez's `Solver.solve` (without the
    /// callback loop — the port has no solve callbacks).
    pub fn solve(&mut self) {
        while self.status() == SolverStatus::Unsolved {
            self.solve_step();
        }
    }

    /// Perform a single solve step. Mirrors rez's `Solver.solve_step`.
    fn solve_step(&mut self) {
        if self.status() != SolverStatus::Unsolved {
            return;
        }

        let mut phase = self.phase_stack.pop().unwrap();

        // A previously failed phase on top: archive it and take the real
        // unsolved phase beneath — this is the implicit backtrack.
        if phase.status == SolverStatus::Failed {
            self.failed_phase_list.push(phase);
            phase = self.phase_stack.pop().unwrap();
        }

        // An exhausted phase must be split before it can be solved further.
        if phase.status == SolverStatus::Exhausted {
            let (first, next) = phase.split();
            self.phase_stack.push(next);
            phase = first;
        }

        let new_phase = phase.solve();
        self.solve_count += 1;

        match new_phase.status {
            SolverStatus::Failed => self.phase_stack.push(new_phase),
            SolverStatus::Solved => {
                // Solved, but may contain a dependency cycle.
                let final_phase = new_phase.finalise();
                self.phase_stack.push(final_phase);
            }
            SolverStatus::Exhausted => self.phase_stack.push(new_phase),
            other => unreachable!("solve() produced unexpected phase status {other:?}"),
        }
    }

    /// The resolved variants, or `None` if the solve did not succeed.
    pub fn resolved_packages(&self) -> Option<Vec<Rc<PackageVariant>>> {
        if self.status() != SolverStatus::Solved {
            return None;
        }
        Some(self.phase_stack.last().unwrap().solved_variants())
    }

    /// Borrowing iterator form of [`Self::resolved_packages`]. `None` if the
    /// solve did not succeed; otherwise an iterator that yields each resolved
    /// variant (cheap `Rc` refcount bumps) without allocating an intermediate
    /// `Vec`.
    pub fn resolved_packages_iter(&self) -> Option<impl Iterator<Item = Rc<PackageVariant>> + '_> {
        if self.status() != SolverStatus::Solved {
            return None;
        }
        Some(self.phase_stack.last().unwrap().iter_solved_variants())
    }

    /// The resolved ephemerals (intersected ranges of every `.foo` requirement
    /// that participated in the solve), or `None` if the solve did not
    /// succeed. Mirrors rez's `Solver.resolved_ephemerals`.
    pub fn resolved_ephemerals(&self) -> Option<Vec<Requirement>> {
        if self.status() != SolverStatus::Solved {
            return None;
        }
        Some(self.phase_stack.last().unwrap().solved_ephemerals())
    }

    /// Borrowing iterator form of [`Self::resolved_ephemerals`]. `None` if the
    /// solve did not succeed; otherwise an iterator that yields each resolved
    /// ephemeral as `&Requirement`, with no intermediate `Vec` and no clones.
    /// The largest saving of the four iterator-form accessors — `Requirement`
    /// clones are heavier than `Rc::clone`.
    pub fn resolved_ephemerals_iter(&self) -> Option<impl Iterator<Item = &Requirement> + '_> {
        if self.status() != SolverStatus::Solved {
            return None;
        }
        Some(self.phase_stack.last().unwrap().iter_solved_ephemerals())
    }

    /// A human-readable failure description, or `None` if the solve did not
    /// fail. Mirrors the gist of rez's `Solver.failure_description`.
    pub fn failure_description(&self) -> Option<String> {
        if self.status() != SolverStatus::Failed {
            return None;
        }
        // The most appropriate failed phase: the current top if it is
        // failed/cyclic, else the first archived failure.
        let top = self.phase_stack.last().unwrap();
        let phase = if matches!(top.status, SolverStatus::Failed | SolverStatus::Cyclic) {
            top
        } else {
            self.failed_phase_list.first().unwrap_or(top)
        };
        Some(match phase.failure_reason() {
            Some(reason) => reason.description(),
            None => "solver failed with unknown reason".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
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
        let map: std::collections::HashMap<String, crate::rez_solver::FamilyMap> = entries
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
            .collect();
        PackageRepo::from_map(map)
    }

    fn solve(repo: PackageRepo, requests: &[&str]) -> Solver {
        let reqs = requests.iter().map(|s| Requirement::parse(s)).collect();
        let mut solver = Solver::new(reqs, Rc::new(repo)).expect("solver construction");
        solver.solve();
        solver
    }

    fn resolved_set(solver: &Solver) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = solver
            .resolved_packages()
            .unwrap()
            .iter()
            .map(|v| (v.name().to_string(), v.version().to_string()))
            .collect();
        out.sort();
        out
    }

    #[test]
    fn test_loader_called_only_for_needed_families() {
        // Two families on disk: "app" depends on "lib". A bystander
        // family "unrelated" exists but the solver never touches it,
        // so the loader must never be called for it.
        use std::cell::RefCell;

        let calls: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let calls_inner = Rc::clone(&calls);

        let repo = crate::rez_solver::PackageRepo::with_loader(Box::new(move |name: &str, _hint: Option<&rer_version::VersionRange>| {
            calls_inner.borrow_mut().push(name.to_string());
            match name {
                "app" => vec![("1.0".to_string(), pkg(&["lib-2"], &[]))],
                "lib" => vec![
                    ("1.0".to_string(), pkg(&[], &[])),
                    ("2.0".to_string(), pkg(&[], &[])),
                ],
                "unrelated" => vec![("1.0".to_string(), pkg(&[], &[]))],
                _ => Vec::new(),
            }
        }));

        let reqs = vec![Requirement::parse("app")];
        let mut solver = Solver::new(reqs, Rc::new(repo)).expect("solver construction");
        solver.solve();
        assert_eq!(solver.status(), SolverStatus::Solved);
        assert_eq!(
            resolved_set(&solver),
            vec![("app".into(), "1.0".into()), ("lib".into(), "2.0".into())]
        );

        let calls = calls.borrow();
        // app and lib were touched; unrelated was not.
        assert!(calls.contains(&"app".to_string()));
        assert!(calls.contains(&"lib".to_string()));
        assert!(!calls.contains(&"unrelated".to_string()));
    }

    #[test]
    fn test_loader_called_once_per_family() {
        use std::cell::RefCell;

        let calls: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let calls_inner = Rc::clone(&calls);

        // A diamond: app -> lib & util; util -> lib. lib is reached twice
        // but the loader must only be invoked once.
        let repo = crate::rez_solver::PackageRepo::with_loader(Box::new(move |name: &str, _hint: Option<&rer_version::VersionRange>| {
            calls_inner.borrow_mut().push(name.to_string());
            match name {
                "app" => vec![("1.0".into(), pkg(&["lib", "util"], &[]))],
                "util" => vec![("1.0".into(), pkg(&["lib"], &[]))],
                "lib" => vec![("1.0".into(), pkg(&[], &[]))],
                _ => Vec::new(),
            }
        }));

        let reqs = vec![Requirement::parse("app")];
        let mut solver = Solver::new(reqs, Rc::new(repo)).expect("solver construction");
        solver.solve();
        assert_eq!(solver.status(), SolverStatus::Solved);

        let calls = calls.borrow();
        let lib_calls = calls.iter().filter(|n| *n == "lib").count();
        assert_eq!(
            lib_calls, 1,
            "loader should be called at most once per family"
        );
    }

    #[test]
    fn test_loader_receives_version_range_hint() {
        // Issue #92: the loader should be invoked with the solver's current
        // range constraint as a hint.
        use rer_version::VersionRange;
        use std::cell::RefCell;

        let calls: Rc<RefCell<Vec<(String, Option<VersionRange>)>>> =
            Rc::new(RefCell::new(Vec::new()));
        let calls_inner = Rc::clone(&calls);

        let repo = crate::rez_solver::PackageRepo::with_loader(Box::new(
            move |name: &str, hint: Option<&VersionRange>| {
                calls_inner
                    .borrow_mut()
                    .push((name.to_string(), hint.cloned()));
                match name {
                    "lib" => vec![
                        ("1.0".to_string(), pkg(&[], &[])),
                        ("2.0".to_string(), pkg(&[], &[])),
                        ("3.0".to_string(), pkg(&[], &[])),
                    ],
                    _ => Vec::new(),
                }
            },
        ));

        let reqs = vec![Requirement::parse("lib-2+<3")];
        let mut solver = Solver::new(reqs, Rc::new(repo)).expect("solver construction");
        solver.solve();
        assert_eq!(solver.status(), SolverStatus::Solved);

        let calls = calls.borrow();
        assert_eq!(calls.len(), 1, "loader called more times than expected");
        let (name, hint) = &calls[0];
        assert_eq!(name, "lib");
        let hint = hint.as_ref().expect("loader should have seen a hint");
        assert_eq!(hint.to_string(), "2+<3");
    }

    #[test]
    fn test_loader_eager_seed_no_loader_call() {
        // A pre-seeded family must not trigger the loader, even when
        // a range-hint request is made for it. The seed counts as a
        // full load.
        use std::cell::RefCell;
        use std::collections::HashMap;

        let calls: Rc<RefCell<usize>> = Rc::new(RefCell::new(0));
        let calls_inner = Rc::clone(&calls);

        let repo = crate::rez_solver::PackageRepo::with_loader(Box::new(
            move |_name: &str, _hint: Option<&rer_version::VersionRange>| {
                *calls_inner.borrow_mut() += 1;
                Vec::new()
            },
        ));
        let mut fam: HashMap<String, _> = HashMap::new();
        fam.insert("1.0".into(), pkg(&[], &[]));
        fam.insert("2.0".into(), pkg(&[], &[]));
        repo.insert_family("lib".into(), fam);

        let reqs = vec![Requirement::parse("lib-2")];
        let mut solver = Solver::new(reqs, Rc::new(repo)).expect("solver construction");
        solver.solve();
        assert_eq!(solver.status(), SolverStatus::Solved);
        assert_eq!(
            *calls.borrow(),
            0,
            "loader must not be called for pre-seeded families"
        );
    }

    #[test]
    fn test_loader_empty_means_missing_family() {
        // The loader returns no entries for an unknown name; the solver
        // treats that as a missing family (failed resolve), not a panic.
        let repo = crate::rez_solver::PackageRepo::with_loader(Box::new(|_: &str, _: Option<&rer_version::VersionRange>| Vec::new()));
        let reqs = vec![Requirement::parse("doesnotexist")];
        let solver = Solver::new(reqs, Rc::new(repo));
        // Either Solver::new returns a ScopeError or the solve fails;
        // both are valid encodings of "no such top-level family".
        match solver {
            Err(_) => {} // expected
            Ok(mut solver) => {
                solver.solve();
                assert_ne!(solver.status(), SolverStatus::Solved);
            }
        }
    }

    #[test]
    fn test_trivial_single_package() {
        let solver = solve(repo(vec![("foo", vec![("1.0", pkg(&[], &[]))])]), &["foo"]);
        assert_eq!(solver.status(), SolverStatus::Solved);
        assert_eq!(resolved_set(&solver), vec![("foo".into(), "1.0".into())]);
    }

    #[test]
    fn test_picks_highest_version() {
        let solver = solve(
            repo(vec![(
                "foo",
                vec![
                    ("1.0", pkg(&[], &[])),
                    ("2.0", pkg(&[], &[])),
                    ("1.5", pkg(&[], &[])),
                ],
            )]),
            &["foo"],
        );
        assert_eq!(resolved_set(&solver), vec![("foo".into(), "2.0".into())]);
    }

    #[test]
    fn test_transitive_dependency() {
        let solver = solve(
            repo(vec![
                ("app", vec![("1.0", pkg(&["lib-2"], &[]))]),
                (
                    "lib",
                    vec![("1.0", pkg(&[], &[])), ("2.0", pkg(&["base"], &[]))],
                ),
                ("base", vec![("3.0", pkg(&[], &[]))]),
            ]),
            &["app"],
        );
        assert_eq!(solver.status(), SolverStatus::Solved);
        assert_eq!(
            resolved_set(&solver),
            vec![
                ("app".into(), "1.0".into()),
                ("base".into(), "3.0".into()),
                ("lib".into(), "2.0".into()),
            ]
        );
    }

    #[test]
    fn test_version_constraint_intersection() {
        // app needs lib-1+<3, other needs lib<2  =>  lib-1.x
        let solver = solve(
            repo(vec![
                ("app", vec![("1.0", pkg(&["lib-1+<3"], &[]))]),
                ("other", vec![("1.0", pkg(&["lib<2"], &[]))]),
                (
                    "lib",
                    vec![
                        ("1.0", pkg(&[], &[])),
                        ("2.0", pkg(&[], &[])),
                        ("3.0", pkg(&[], &[])),
                    ],
                ),
            ]),
            &["app", "other"],
        );
        assert_eq!(solver.status(), SolverStatus::Solved);
        assert_eq!(
            resolved_set(&solver),
            vec![
                ("app".into(), "1.0".into()),
                ("lib".into(), "1.0".into()),
                ("other".into(), "1.0".into()),
            ]
        );
    }

    #[test]
    fn test_conflict_request_fails() {
        // foo-1 and foo-2 cannot both be satisfied.
        let solver = solve(
            repo(vec![(
                "foo",
                vec![("1.0", pkg(&[], &[])), ("2.0", pkg(&[], &[]))],
            )]),
            &["foo-1", "foo-2"],
        );
        assert_eq!(solver.status(), SolverStatus::Failed);
        assert!(solver.resolved_packages().is_none());
    }

    #[test]
    fn test_unsatisfiable_transitive_fails() {
        // app needs lib-5, but only lib-1 exists.
        let solver = solve(
            repo(vec![
                ("app", vec![("1.0", pkg(&["lib-5"], &[]))]),
                ("lib", vec![("1.0", pkg(&[], &[]))]),
            ]),
            &["app"],
        );
        assert_eq!(solver.status(), SolverStatus::Failed);
    }

    #[test]
    fn test_conflict_requirement_excludes_version() {
        // app pulls lib (any); !lib-2 forbids lib-2  => lib-1.0
        let solver = solve(
            repo(vec![
                ("app", vec![("1.0", pkg(&["lib"], &[]))]),
                ("lib", vec![("1.0", pkg(&[], &[])), ("2.0", pkg(&[], &[]))]),
            ]),
            &["app", "!lib-2"],
        );
        assert_eq!(solver.status(), SolverStatus::Solved);
        assert_eq!(
            resolved_set(&solver),
            vec![("app".into(), "1.0".into()), ("lib".into(), "1.0".into())],
        );
    }

    #[test]
    fn test_weak_reference_does_not_pull_in() {
        // ~lib-1 must NOT pull lib into the resolve on its own.
        let solver = solve(
            repo(vec![
                ("foo", vec![("1.0", pkg(&[], &[]))]),
                ("lib", vec![("1.0", pkg(&[], &[]))]),
            ]),
            &["foo", "~lib-1"],
        );
        assert_eq!(solver.status(), SolverStatus::Solved);
        // only foo is resolved — lib was not requested, only weakly referenced
        assert_eq!(resolved_set(&solver), vec![("foo".into(), "1.0".into())]);
    }

    #[test]
    fn test_iter_resolved_packages_matches_vec_form() {
        let solver = solve(
            repo(vec![
                ("app", vec![("1.0", pkg(&["lib-2"], &[]))]),
                ("lib", vec![("1.0", pkg(&[], &[])), ("2.0", pkg(&[], &[]))]),
            ]),
            &["app"],
        );
        let owned: Vec<(String, String)> = solver
            .resolved_packages()
            .unwrap()
            .iter()
            .map(|v| (v.name().to_string(), v.version().to_string()))
            .collect();
        let borrowed: Vec<(String, String)> = solver
            .resolved_packages_iter()
            .unwrap()
            .map(|v| (v.name().to_string(), v.version().to_string()))
            .collect();
        assert_eq!(owned, borrowed);
        // Iter form on a failed solve returns None too.
        let failed = solve(
            repo(vec![(
                "foo",
                vec![("1.0", pkg(&[], &[])), ("2.0", pkg(&[], &[]))],
            )]),
            &["foo-1", "foo-2"],
        );
        assert!(failed.resolved_packages_iter().is_none());
    }

    #[test]
    fn test_iter_resolved_ephemerals_matches_vec_form() {
        let solver = solve(
            repo(vec![("foo", vec![("1.0", pkg(&[], &[]))])]),
            &["foo", ".feature-1+<3", ".feature-2+"],
        );
        let owned: Vec<String> = solver
            .resolved_ephemerals()
            .unwrap()
            .iter()
            .map(|r| r.to_string())
            .collect();
        let borrowed: Vec<String> = solver
            .resolved_ephemerals_iter()
            .unwrap()
            .map(|r| r.to_string())
            .collect();
        assert_eq!(owned, borrowed);
        // The borrowing form yields `&Requirement` — confirm callers can
        // collect without forcing an owned copy of the requirements.
        let by_ref: Vec<&Requirement> = solver.resolved_ephemerals_iter().unwrap().collect();
        assert_eq!(by_ref.len(), 1);
        assert_eq!(by_ref[0].to_string(), ".feature-2+<3");
    }

    #[test]
    fn test_resolved_ephemerals_intersect_request_only() {
        // Two ephemerals on the request intersect to the narrower range.
        let solver = solve(
            repo(vec![("foo", vec![("1.0", pkg(&[], &[]))])]),
            &["foo", ".feature-1+<3", ".feature-2+"],
        );
        assert_eq!(solver.status(), SolverStatus::Solved);
        let eph: Vec<String> = solver
            .resolved_ephemerals()
            .expect("solved => Some")
            .iter()
            .map(|r| r.to_string())
            .collect();
        assert_eq!(eph, vec![".feature-2+<3".to_string()]);
    }

    #[test]
    fn test_resolved_ephemerals_from_package_requires() {
        // The ephemeral is contributed by a resolved package's `requires`.
        let solver = solve(
            repo(vec![("app", vec![("1.0", pkg(&[".mode-debug"], &[]))])]),
            &["app"],
        );
        assert_eq!(solver.status(), SolverStatus::Solved);
        let eph: Vec<String> = solver
            .resolved_ephemerals()
            .expect("solved => Some")
            .iter()
            .map(|r| r.to_string())
            .collect();
        assert_eq!(eph, vec![".mode-debug".to_string()]);
    }

    #[test]
    fn test_resolved_ephemerals_empty_when_none_in_solve() {
        let solver = solve(repo(vec![("foo", vec![("1.0", pkg(&[], &[]))])]), &["foo"]);
        assert_eq!(solver.status(), SolverStatus::Solved);
        assert_eq!(
            solver.resolved_ephemerals().expect("solved => Some"),
            Vec::<Requirement>::new()
        );
    }

    #[test]
    fn test_resolved_ephemerals_none_when_not_solved() {
        let solver = solve(
            repo(vec![(
                "foo",
                vec![("1.0", pkg(&[], &[])), ("2.0", pkg(&[], &[]))],
            )]),
            &["foo-1", "foo-2"],
        );
        assert_eq!(solver.status(), SolverStatus::Failed);
        assert!(solver.resolved_ephemerals().is_none());
    }

    #[test]
    fn test_variant_selection() {
        // foo has two variants; the solver should pick one and resolve its deps.
        let solver = solve(
            repo(vec![
                (
                    "foo",
                    vec![("1.0", pkg(&[], &[&["maya-2024"], &["maya-2025"]]))],
                ),
                (
                    "maya",
                    vec![("2024.0", pkg(&[], &[])), ("2025.0", pkg(&[], &[]))],
                ),
            ]),
            &["foo"],
        );
        assert_eq!(solver.status(), SolverStatus::Solved);
        let set = resolved_set(&solver);
        assert!(set.contains(&("foo".into(), "1.0".into())));
        // exactly one maya variant was chosen
        assert_eq!(set.iter().filter(|(n, _)| n == "maya").count(), 1);
    }

    #[test]
    fn test_cycle_detection() {
        // a -> b -> a is a dependency cycle.
        let solver = solve(
            repo(vec![
                ("a", vec![("1.0", pkg(&["b"], &[]))]),
                ("b", vec![("1.0", pkg(&["a"], &[]))]),
            ]),
            &["a"],
        );
        assert_eq!(solver.status(), SolverStatus::Failed);
    }
}
