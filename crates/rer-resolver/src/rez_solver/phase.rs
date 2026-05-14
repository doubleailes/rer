//! `_ResolvePhase` — a full copy of the resolve state, run until no further
//! progress is possible without selecting a sub-range. Ported from
//! `rez/src/rez/solver.py` (`_ResolvePhase`).
//!
//! The algorithm is the cycle: **extract** a dependency common to all of a
//! scope's variants → **merge** the extractions → **intersect** them into the
//! existing scopes → **add** scopes for new families → **reduce** every scope
//! against every (changed) scope. Repeat until stable. If a scope still has
//! multiple variants, the phase is `Exhausted` and must be `split`.

use super::context::SolverContext;
use super::failure::{DependencyConflict, FailureReason, SolverStatus};
use super::graph::DepGraph;
use super::requirement::{Requirement, RequirementList};
use super::scope::{PackageScope, ScopeError, ScopeIntersect, ScopeReduce};
use super::variant::PackageVariant;
use rustc_hash::{FxHashMap, FxHashSet};
use std::rc::Rc;

/// A full copy of the resolve state. Mirrors rez's `_ResolvePhase`.
#[derive(Debug, Clone)]
pub struct ResolvePhase {
    ctx: Rc<SolverContext>,
    /// One scope per package family currently in the resolve.
    ///
    /// Scopes are `Rc`-shared: rez treats scopes as immutable (replaced, never
    /// mutated) and shares the objects across phases by reference, so cloning a
    /// phase — which happens constantly during backtracking — must be cheap.
    scopes: Vec<Rc<PackageScope>>,
    /// Why the phase failed, if it did.
    failure_reason: Option<FailureReason>,
    /// `(scope_name, extracted_name) -> requirement` — every extraction so far.
    extractions: FxHashMap<(String, String), Requirement>,
    /// Indices of scopes changed since the last reduce — seeds the next reduce.
    changed_scopes_i: FxHashSet<usize>,
    /// The phase's current status.
    pub status: SolverStatus,
}

impl ResolvePhase {
    /// Build the initial phase from the solver context's request list. Fails
    /// if a top-level request names a missing package.
    pub fn new(ctx: &Rc<SolverContext>) -> Result<Self, ScopeError> {
        let mut scopes = Vec::new();
        for req in ctx.request_list.iter() {
            scopes.push(Rc::new(PackageScope::new(req.clone(), ctx)?));
        }
        let n = scopes.len();
        Ok(ResolvePhase {
            ctx: Rc::clone(ctx),
            scopes,
            failure_reason: None,
            extractions: FxHashMap::default(),
            // Force an initial all-pairs reduction in a fresh phase.
            changed_scopes_i: (0..n).collect(),
            status: SolverStatus::Pending,
        })
    }

    /// A phase pre-marked as failed (used for a conflicting top-level request).
    pub fn failed(ctx: &Rc<SolverContext>, failure_reason: FailureReason) -> Self {
        ResolvePhase {
            ctx: Rc::clone(ctx),
            scopes: Vec::new(),
            failure_reason: Some(failure_reason),
            extractions: FxHashMap::default(),
            changed_scopes_i: FxHashSet::default(),
            status: SolverStatus::Failed,
        }
    }

    /// The phase's failure reason, if any.
    pub fn failure_reason(&self) -> Option<&FailureReason> {
        self.failure_reason.as_ref()
    }

    /// True if every scope is solved.
    fn is_solved(scopes: &[Rc<PackageScope>]) -> bool {
        scopes.iter().all(|s| s.is_solved())
    }

    /// Assemble a result phase, mirroring rez's `_create_phase`. `status: None`
    /// resolves to `Solved`/`Exhausted` from the scopes.
    fn make_phase(
        &self,
        scopes: Vec<Rc<PackageScope>>,
        failure_reason: Option<FailureReason>,
        extractions: FxHashMap<(String, String), Requirement>,
        status: Option<SolverStatus>,
    ) -> ResolvePhase {
        let status = status.unwrap_or_else(|| {
            if Self::is_solved(&scopes) {
                SolverStatus::Solved
            } else {
                SolverStatus::Exhausted
            }
        });
        ResolvePhase {
            ctx: Rc::clone(&self.ctx),
            scopes,
            failure_reason,
            extractions,
            changed_scopes_i: FxHashSet::default(),
            status,
        }
    }

    /// Run the resolve algorithm on this phase, returning the resulting phase
    /// (`Solved`, `Exhausted`, or `Failed`).
    pub fn solve(&self) -> ResolvePhase {
        if self.status != SolverStatus::Pending {
            return self.clone();
        }

        let mut scopes = self.scopes.clone();
        let mut extractions = FxHashMap::default();
        let mut changed_scopes_i = self.changed_scopes_i.clone();

        // Outer loop: iteratively reduce until no more reductions are possible.
        loop {
            let prev_num_scopes = scopes.len();
            let mut widened_scopes_i: FxHashSet<usize> = FxHashSet::default();

            // Inner loop: iteratively extract until no more extractions.
            loop {
                let mut extracted_requests: Vec<Requirement> = Vec::new();

                // EXTRACT: pull every common dependency from every scope.
                for i in 0..scopes.len() {
                    loop {
                        match scopes[i].extract() {
                            Some((scope_, extracted_request)) => {
                                let key = (
                                    scopes[i].package_name().to_string(),
                                    extracted_request.name().to_string(),
                                );
                                extractions.insert(key, extracted_request.clone());
                                extracted_requests.push(extracted_request);
                                scopes[i] = Rc::new(scope_);
                            }
                            None => break,
                        }
                    }
                }

                if extracted_requests.is_empty() {
                    break;
                }

                // MERGE-EXTRACTIONS: there may be overlaps.
                let extracted_requests = RequirementList::new(extracted_requests);
                if let Some((req1, req2)) = extracted_requests.conflict() {
                    let failure = FailureReason::DependencyConflicts(vec![DependencyConflict {
                        dependency: req1.clone(),
                        conflicting_request: req2.clone(),
                    }]);
                    return self.make_phase(
                        scopes,
                        Some(failure),
                        extractions,
                        Some(SolverStatus::Failed),
                    );
                }

                // INTERSECT extracted requests into the existing scopes.
                let mut req_fams: FxHashSet<String> = FxHashSet::default();
                for i in 0..scopes.len() {
                    let extracted_req = match extracted_requests.get(scopes[i].package_name()) {
                        Some(req) => req.clone(),
                        None => continue,
                    };
                    req_fams.insert(extracted_req.name().to_string());

                    let was_conflict = scopes[i].is_conflict();
                    let range = extracted_req
                        .range()
                        .expect("an extracted requirement always has a range");
                    match scopes[i].intersect(range) {
                        ScopeIntersect::Empty => {
                            let failure =
                                FailureReason::DependencyConflicts(vec![DependencyConflict {
                                    dependency: extracted_req,
                                    conflicting_request: scopes[i]
                                        .package_request()
                                        .expect("scope has a package request")
                                        .clone(),
                                }]);
                            return self.make_phase(
                                scopes,
                                Some(failure),
                                extractions,
                                Some(SolverStatus::Failed),
                            );
                        }
                        ScopeIntersect::Unchanged => {}
                        ScopeIntersect::Narrowed(scope_) => {
                            let now_conflict = scope_.is_conflict();
                            scopes[i] = Rc::new(scope_);
                            changed_scopes_i.insert(i);
                            // A conflict scope that became a normal scope has
                            // *widened* — it must reduce against everything.
                            if was_conflict && !now_conflict {
                                widened_scopes_i.insert(i);
                            }
                        }
                    }
                }

                // ADD scopes for extracted families with no existing scope.
                let new_reqs: Vec<Requirement> = extracted_requests
                    .requirements()
                    .iter()
                    .filter(|r| !req_fams.contains(r.name()))
                    .cloned()
                    .collect();
                for req in new_reqs {
                    match PackageScope::new(req, &self.ctx) {
                        Ok(scope) => scopes.push(Rc::new(scope)),
                        Err(err) => {
                            // rez raises here by default; the benchmark records
                            // no "error" resolves, so treat an unresolvable
                            // newly-added family as a phase failure.
                            let failure =
                                FailureReason::DependencyConflicts(vec![DependencyConflict {
                                    dependency: Requirement::parse(match &err {
                                        ScopeError::FamilyNotFound(n) => n,
                                        ScopeError::PackageNotFound(r) => r.name(),
                                    }),
                                    conflicting_request: match &err {
                                        ScopeError::FamilyNotFound(n) => Requirement::parse(n),
                                        ScopeError::PackageNotFound(r) => r.clone(),
                                    },
                                }]);
                            return self.make_phase(
                                scopes,
                                Some(failure),
                                extractions,
                                Some(SolverStatus::Failed),
                            );
                        }
                    }
                }
            }

            let num_scopes = scopes.len();

            // No new scopes and nothing changed — the phase is fully reduced.
            if num_scopes == prev_num_scopes
                && changed_scopes_i.is_empty()
                && widened_scopes_i.is_empty()
            {
                break;
            }

            // REDUCE: build the pending (x, y) reduction pairs — "reduce
            // scope[x] by scope[y].package_request".
            let mut pending_set: FxHashSet<(usize, usize)> = FxHashSet::default();
            // existing scopes reduce against changed scopes
            for x in 0..prev_num_scopes {
                for &y in &changed_scopes_i {
                    pending_set.insert((x, y));
                }
            }
            // existing scopes reduce against newly added scopes
            for x in 0..prev_num_scopes {
                for y in prev_num_scopes..num_scopes {
                    pending_set.insert((x, y));
                }
            }
            // newly added scopes reduce against all scopes
            for x in prev_num_scopes..num_scopes {
                for y in 0..num_scopes {
                    pending_set.insert((x, y));
                }
            }
            // widened scopes reduce against all scopes
            for &x in &widened_scopes_i {
                for y in 0..num_scopes {
                    pending_set.insert((x, y));
                }
            }

            // Sort for determinism, then process as a LIFO stack (rez sorts the
            // set into a list and `pop()`s from the end).
            let mut pending: Vec<(usize, usize)> = pending_set.into_iter().collect();
            pending.sort_unstable();

            while let Some((x, y)) = pending.pop() {
                if x == y {
                    continue;
                }
                let request = scopes[y]
                    .package_request()
                    .expect("scope has a package request")
                    .clone();
                let (result, reductions) = scopes[x].reduce_by(&request);
                match result {
                    ScopeReduce::Empty => {
                        let failure = FailureReason::TotalReduction(reductions);
                        return self.make_phase(
                            scopes,
                            Some(failure),
                            extractions,
                            Some(SolverStatus::Failed),
                        );
                    }
                    ScopeReduce::Reduced(new_scope) => {
                        scopes[x] = Rc::new(new_scope);
                        // Every other scope must reduce against the narrower x.
                        for j in 0..num_scopes {
                            if j != x {
                                pending.push((j, x));
                            }
                        }
                    }
                    ScopeReduce::Unchanged => {}
                }
            }

            changed_scopes_i.clear();
        }

        self.make_phase(scopes, None, extractions, None)
    }

    /// Remove conflict scopes, detect dependency cycles, and reorder the
    /// resolved packages (children before parents, near request order).
    /// Mirrors rez's `_ResolvePhase.finalise`.
    pub fn finalise(&self) -> ResolvePhase {
        debug_assert!(Self::is_solved(&self.scopes));

        // Work on a clone — `get_solved_variant` mutates (sorts) its scope.
        let mut scopes = self.scopes.clone();

        // Build the minimal dependency graph over the solved (non-conflict)
        // scopes, and a name -> index map into `scopes`.
        let mut graph = DepGraph::new();
        let mut index_by_name: FxHashMap<String, usize> = FxHashMap::default();
        for i in 0..scopes.len() {
            if scopes[i].is_conflict() {
                continue;
            }
            let name = scopes[i].package_name().to_string();
            graph.add_node(name.clone());
            index_by_name.insert(name.clone(), i);
            if let Some(variant) = Rc::make_mut(&mut scopes[i]).get_solved_variant() {
                for req in variant.requires().iter() {
                    if !req.is_conflict() {
                        graph.add_edge(name.clone(), req.name().to_string());
                    }
                }
            }
        }

        // Cycle detection.
        let cycle_fams = graph.find_cycle();
        if !cycle_fams.is_empty() {
            let mut cycle: Vec<(String, rer_version::RerVersion)> = Vec::new();
            for fam in &cycle_fams {
                let idx = index_by_name[fam];
                let version = Rc::make_mut(&mut scopes[idx])
                    .get_solved_variant()
                    .expect("cycle node is a solved scope")
                    .version()
                    .clone();
                cycle.push((fam.clone(), version));
            }
            let final_scopes: Vec<Rc<PackageScope>> =
                scopes.into_iter().filter(|s| !s.is_conflict()).collect();
            return ResolvePhase {
                ctx: Rc::clone(&self.ctx),
                scopes: final_scopes,
                failure_reason: Some(FailureReason::Cycle(cycle)),
                extractions: self.extractions.clone(),
                changed_scopes_i: FxHashSet::default(),
                status: SolverStatus::Cyclic,
            };
        }

        // Reorder: children before parents, staying near request order.
        let request_fams: Vec<String> = self
            .ctx
            .request_list
            .iter()
            .map(|r| r.name().to_string())
            .collect();
        let ordered_fams = graph.dependency_order(&request_fams);

        // Pull the (non-conflict) scopes out in dependency order.
        let mut taken: Vec<Option<Rc<PackageScope>>> = scopes.into_iter().map(Some).collect();
        let mut final_scopes: Vec<Rc<PackageScope>> = Vec::new();
        for fam in ordered_fams {
            if let Some(&idx) = index_by_name.get(&fam) {
                if let Some(scope) = taken[idx].take() {
                    if !scope.is_conflict() {
                        final_scopes.push(scope);
                    }
                }
            }
        }

        ResolvePhase {
            ctx: Rc::clone(&self.ctx),
            scopes: final_scopes,
            failure_reason: None,
            extractions: self.extractions.clone(),
            changed_scopes_i: FxHashSet::default(),
            status: self.status,
        }
    }

    /// Split an exhausted phase into a preferred half (whose split scope now
    /// has an extractable common dependency) and the remainder. Mirrors rez's
    /// `_ResolvePhase.split`.
    pub fn split(&self) -> (ResolvePhase, ResolvePhase) {
        debug_assert_eq!(self.status, SolverStatus::Exhausted);

        let mut scopes: Vec<Rc<PackageScope>> = Vec::with_capacity(self.scopes.len());
        let mut next_scopes: Vec<Rc<PackageScope>> = Vec::with_capacity(self.scopes.len());
        let mut split_i: Option<usize> = None;

        for (i, scope) in self.scopes.iter().enumerate() {
            if split_i.is_none() {
                let mut candidate = (**scope).clone();
                if let Some((scope_, next_scope)) = candidate.split() {
                    scopes.push(Rc::new(scope_));
                    next_scopes.push(Rc::new(next_scope));
                    split_i = Some(i);
                    continue;
                }
            }
            // Unchanged scopes are shared by reference between the two phases.
            scopes.push(Rc::clone(scope));
            next_scopes.push(Rc::clone(scope));
        }

        let split_i = split_i.expect("an exhausted phase always has a splittable scope");

        let phase = ResolvePhase {
            ctx: Rc::clone(&self.ctx),
            scopes,
            failure_reason: None,
            extractions: self.extractions.clone(),
            changed_scopes_i: [split_i].into_iter().collect(),
            status: SolverStatus::Pending,
        };
        let next_phase = ResolvePhase {
            scopes: next_scopes,
            ..phase.clone()
        };
        (phase, next_phase)
    }

    /// The solved variants of this (solved) phase, in scope order.
    pub fn solved_variants(&self) -> Vec<Rc<PackageVariant>> {
        let mut scopes = self.scopes.clone();
        scopes
            .iter_mut()
            .filter_map(|s| Rc::make_mut(s).get_solved_variant())
            .collect()
    }
}

impl std::fmt::Display for ResolvePhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let parts: Vec<String> = self.scopes.iter().map(|s| s.to_string()).collect();
        write!(f, "{}", parts.join(" "))
    }
}
