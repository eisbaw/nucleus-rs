//! Canonical host-election rule shared by every tier-1 backend's
//! `multi_worker::Plan::build` AND by the compiler-level passes in
//! `nucleus-driver` that mediate against the backend-elected host
//! (cycles 160 / 162 / 163 — `host_mediation_inject`,
//! `safe_push_reorder`, `host_data_relay_inject`).
//!
//! # The rule (load-bearing — every site MUST use these helpers,
//! not approximate the logic inline)
//!
//! 1. Prefer the worker literally named `"host"` AND in `used`.
//! 2. Else fall back to the smallest used `WorkerId`
//!    (i.e. `used.first()` for the sorted-ascending `Vec`/`BTreeSet`).
//! 3. If `used` is empty, return `None`. Each caller handles `None`
//!    differently:
//!    - backends raise [`EmitError::ContractGap`];
//!    - driver wirings pass the ACFG / per-worker through unchanged.
//!
//! # Why a shared helper exists (TASK-0336 cycle 164)
//!
//! Cycle-160 architect P1.1 + memory
//! `feedback-driver-must-mirror-backend-election-exactly`: a
//! compiler-level driver pass mediating against a backend-elected
//! entity (host worker) MUST use the same election rule as the
//! backend, not an approximation, or latent cross-backend skew
//! leaks into the bit-identical differential (PRD §10.1).
//!
//! Through cycles 160 (host_mediation_inject), 162 (safe_push_reorder),
//! 163 (host_data_relay_inject) the driver accumulated three
//! independent in-line mirrorings of the rule. Cycle-163b architect
//! P2.5 fold-back filed TASK-0336 to retire the recurrence surface:
//! lift the rule to one helper, all 7 production sites consume it,
//! a 4th driver wiring or a future refactor of `Plan::build` cannot
//! silently drift.
//!
//! # Two view variants
//!
//! Backends iterate
//! [`NameTables::worker`](nucleus_compiler::NameTables::worker), which
//! is `BTreeMap<WorkerId, String>` (key = WorkerId, value = name).
//! The driver iterates `ACFG::name_workers`, which is
//! `BTreeMap<String, WorkerId>` (key = name, value = WorkerId).
//! Rather than allocating a flipped map at one of the sites (which
//! adds latency to every codegen run) or hand-rolling a single
//! closure-based generic (which adds boilerplate at every callsite),
//! this module exposes two thin public wrappers — one per view —
//! that both delegate to a single private core. The core is the
//! one place the rule lives.
//!
//! # `used` sort invariant (load-bearing)
//!
//! Both callsites construct `used` from a `BTreeMap`'s keys in
//! iteration order, which is ascending. Backends collect into a
//! `Vec<WorkerId>` (and the helper accepts `&[WorkerId]`); drivers
//! collect into a `BTreeSet<WorkerId>` (and the helper accepts
//! `&BTreeSet<WorkerId>`). For both, "smallest used WorkerId" is
//! the first element (`.first()` / `.iter().next()`). If a caller
//! ever feeds an unsorted `&[WorkerId]` slice to
//! [`elect_host_from_worker_names`], the fallback branch will pick
//! the FIRST element of the slice, not the smallest — silently
//! wrong. The slice MUST be sorted ascending.

use std::collections::{BTreeMap, BTreeSet};

use nucleus_compiler::event::WorkerId;

/// The reserved worker name that wins host election when present
/// in `used`. Single source of truth for the literal — backends
/// and driver wirings consume this constant via the
/// `elect_host_from_*` helpers below; no production codegen site
/// should compare against the literal `"host"` directly.
///
/// Test fixtures and assertions on emitted code may still mention
/// the string literal verbatim — those are not election sites.
pub const HOST_NAME: &str = "host";

/// Apply the canonical host-election rule once the caller has
/// pre-computed the two candidates from its view:
///
/// - `named_host_in_used`: the WorkerId whose name is
///   [`HOST_NAME`] AND that is present in `used`, else `None`.
/// - `smallest_used`: the smallest `WorkerId` in `used`, else
///   `None` when `used` is empty.
///
/// Returns `named_host_in_used.or(smallest_used)` — see the module
/// docstring for the full rule and the meaning of `None`.
///
/// Private: the only intended callers are the two public wrappers
/// in this module. Exposing the core directly would invite callers
/// to hand-compute the two candidates with subtly-different
/// criteria (e.g. case-insensitive name match), defeating the
/// canonicalisation.
fn elect_host_core(
    named_host_in_used: Option<WorkerId>,
    smallest_used: Option<WorkerId>,
) -> Option<WorkerId> {
    named_host_in_used.or(smallest_used)
}

/// Backend view of the canonical host-election rule. Use this in
/// every tier-1 backend's `multi_worker::Plan::build`.
///
/// `worker_names` is the [`NameTables::worker`] map (WorkerId ->
/// name). `used_sorted_asc` is the slice of `WorkerId`s with
/// non-empty event lists, in ascending `WorkerId` order — backends
/// build this from `per_worker.iter().filter(...)`, and BTreeMap
/// iteration yields keys ascending, so the invariant is preserved
/// by construction.
///
/// Returns the elected host or `None` if `used_sorted_asc` is
/// empty. The caller is responsible for raising
/// [`EmitError::ContractGap`] on `None`.
pub fn elect_host_from_worker_names(
    worker_names: &BTreeMap<WorkerId, String>,
    used_sorted_asc: &[WorkerId],
) -> Option<WorkerId> {
    // Cycle-164b architect P3.1: the docstring's "sorted ascending"
    // invariant is load-bearing — `first()` returns "smallest used"
    // only if the slice is sorted. All current callers build the
    // slice from `BTreeMap::keys()` (sorted by construction), but a
    // future caller passing an unsorted Vec would silently elect the
    // FIRST element. Debug-assert it; zero cost in release.
    debug_assert!(
        used_sorted_asc.windows(2).all(|w| w[0] < w[1]),
        "elect_host_from_worker_names: used_sorted_asc must be \
         strictly ascending by WorkerId (got {used_sorted_asc:?})",
    );
    let named_host_in_used = worker_names
        .iter()
        .find(|(_, n)| n.as_str() == HOST_NAME)
        .map(|(w, _)| *w)
        .filter(|w| used_sorted_asc.contains(w));
    elect_host_core(named_host_in_used, used_sorted_asc.first().copied())
}

/// Driver view of the canonical host-election rule. Use this in
/// every compiler-level pass in `nucleus-driver` that mediates
/// against the backend-elected host worker (cycles 160 / 162 / 163).
///
/// `name_workers` is the `ACFG::name_workers` map (name ->
/// WorkerId). `used` is the set of `WorkerId`s with non-empty
/// event lists; `BTreeSet` iteration yields ascending order so
/// "smallest used" is `used.iter().next()`.
///
/// Returns the elected host or `None` if `used` is empty. Driver
/// wirings typically interpret `None` as "degenerate ACFG, pass
/// through unchanged" (cf. `apply_host_mediation_inject`,
/// `apply_host_data_relay_inject`, `apply_safe_push_reorder`).
pub fn elect_host_from_name_workers(
    name_workers: &BTreeMap<String, WorkerId>,
    used: &BTreeSet<WorkerId>,
) -> Option<WorkerId> {
    let named_host_in_used = name_workers
        .get(HOST_NAME)
        .copied()
        .filter(|w| used.contains(w));
    elect_host_core(named_host_in_used, used.iter().next().copied())
}

#[cfg(test)]
mod tests {
    //! Per TASK-0336 AC#2: parameterised regression pin for every
    //! branch of the rule, exercised through BOTH public wrappers so
    //! a divergent re-inlining at one site (the very pattern this
    //! refactor retires) would not regress unnoticed.

    use super::*;

    fn w(id: u64) -> WorkerId {
        WorkerId(id)
    }

    fn worker_names_with(pairs: &[(u64, &str)]) -> BTreeMap<WorkerId, String> {
        pairs
            .iter()
            .map(|(i, n)| (w(*i), (*n).to_string()))
            .collect()
    }

    fn name_workers_with(pairs: &[(&str, u64)]) -> BTreeMap<String, WorkerId> {
        pairs
            .iter()
            .map(|(n, i)| ((*n).to_string(), w(*i)))
            .collect()
    }

    // --- Backend view (worker_names: WorkerId -> name) ---

    #[test]
    fn backend_view_named_host_present_and_in_used_wins() {
        // Named host exists at WorkerId(5) and is in used; even
        // though WorkerId(1) is smaller and also in used, the named
        // host wins by rule #1.
        let names = worker_names_with(&[(1, "w0"), (5, "host"), (9, "w1")]);
        let used = vec![w(1), w(5), w(9)];
        assert_eq!(elect_host_from_worker_names(&names, &used), Some(w(5)));
    }

    #[test]
    fn backend_view_named_host_present_but_not_in_used_falls_back_to_smallest() {
        // Named host exists at WorkerId(5) but is NOT in used; rule
        // #2 falls back to the smallest used WorkerId.
        let names = worker_names_with(&[(1, "w0"), (5, "host"), (9, "w1")]);
        let used = vec![w(1), w(9)];
        assert_eq!(elect_host_from_worker_names(&names, &used), Some(w(1)));
    }

    #[test]
    fn backend_view_no_named_host_falls_back_to_smallest_used() {
        // No worker is named "host"; rule #2 picks smallest used.
        let names = worker_names_with(&[(1, "w0"), (5, "w1"), (9, "w2")]);
        let used = vec![w(5), w(9)];
        assert_eq!(elect_host_from_worker_names(&names, &used), Some(w(5)));
    }

    #[test]
    fn backend_view_empty_used_returns_none() {
        // Rule #3: empty `used` -> None. Backend caller raises
        // ContractGap; the helper itself does not panic.
        let names = worker_names_with(&[(1, "w0"), (5, "host")]);
        let used: Vec<WorkerId> = vec![];
        assert_eq!(elect_host_from_worker_names(&names, &used), None);
    }

    #[test]
    fn backend_view_smallest_used_wins_on_tie() {
        // No named host present at all (not just "not in used"):
        // pick the smallest used WorkerId, NOT the first slice
        // entry. The slice is sorted ascending by contract.
        let names = worker_names_with(&[(2, "w0"), (3, "w1"), (7, "w2")]);
        let used = vec![w(2), w(3), w(7)];
        assert_eq!(elect_host_from_worker_names(&names, &used), Some(w(2)));
    }

    // --- Driver view (name_workers: name -> WorkerId) ---

    #[test]
    fn driver_view_named_host_present_and_in_used_wins() {
        let names = name_workers_with(&[("w0", 1), ("host", 5), ("w1", 9)]);
        let mut used = BTreeSet::new();
        used.insert(w(1));
        used.insert(w(5));
        used.insert(w(9));
        assert_eq!(elect_host_from_name_workers(&names, &used), Some(w(5)));
    }

    #[test]
    fn driver_view_named_host_present_but_not_in_used_falls_back_to_smallest() {
        let names = name_workers_with(&[("w0", 1), ("host", 5), ("w1", 9)]);
        let mut used = BTreeSet::new();
        used.insert(w(1));
        used.insert(w(9));
        assert_eq!(elect_host_from_name_workers(&names, &used), Some(w(1)));
    }

    #[test]
    fn driver_view_no_named_host_falls_back_to_smallest_used() {
        let names = name_workers_with(&[("w0", 1), ("w1", 5), ("w2", 9)]);
        let mut used = BTreeSet::new();
        used.insert(w(5));
        used.insert(w(9));
        assert_eq!(elect_host_from_name_workers(&names, &used), Some(w(5)));
    }

    #[test]
    fn driver_view_empty_used_returns_none() {
        let names = name_workers_with(&[("w0", 1), ("host", 5)]);
        let used: BTreeSet<WorkerId> = BTreeSet::new();
        assert_eq!(elect_host_from_name_workers(&names, &used), None);
    }

    #[test]
    fn driver_view_smallest_used_wins_on_tie() {
        let names = name_workers_with(&[("w0", 2), ("w1", 3), ("w2", 7)]);
        let mut used = BTreeSet::new();
        used.insert(w(2));
        used.insert(w(3));
        used.insert(w(7));
        assert_eq!(elect_host_from_name_workers(&names, &used), Some(w(2)));
    }

    // --- Cross-view symmetry (defends against divergent
    //     re-inlining of the rule at one wrapper but not the other,
    //     which is the exact recurrence surface this refactor
    //     retires per memory
    //     feedback-driver-must-mirror-backend-election-exactly) ---

    #[test]
    fn both_views_elect_the_same_host_for_the_same_input() {
        let pairs_by_id: &[(u64, &str)] = &[(1, "w0"), (5, "host"), (9, "w1")];
        let pairs_by_name: &[(&str, u64)] = &[("w0", 1), ("host", 5), ("w1", 9)];

        // Case A: named host in used.
        let used_vec = vec![w(1), w(5), w(9)];
        let mut used_set = BTreeSet::new();
        for u in &used_vec {
            used_set.insert(*u);
        }
        assert_eq!(
            elect_host_from_worker_names(&worker_names_with(pairs_by_id), &used_vec),
            elect_host_from_name_workers(&name_workers_with(pairs_by_name), &used_set),
        );

        // Case B: named host NOT in used.
        let used_vec = vec![w(1), w(9)];
        let mut used_set = BTreeSet::new();
        for u in &used_vec {
            used_set.insert(*u);
        }
        assert_eq!(
            elect_host_from_worker_names(&worker_names_with(pairs_by_id), &used_vec),
            elect_host_from_name_workers(&name_workers_with(pairs_by_name), &used_set),
        );

        // Case C: no named host present.
        let pairs_by_id_no_host: &[(u64, &str)] = &[(1, "w0"), (5, "w1"), (9, "w2")];
        let pairs_by_name_no_host: &[(&str, u64)] = &[("w0", 1), ("w1", 5), ("w2", 9)];
        let used_vec = vec![w(5), w(9)];
        let mut used_set = BTreeSet::new();
        for u in &used_vec {
            used_set.insert(*u);
        }
        assert_eq!(
            elect_host_from_worker_names(&worker_names_with(pairs_by_id_no_host), &used_vec),
            elect_host_from_name_workers(&name_workers_with(pairs_by_name_no_host), &used_set),
        );

        // Case D: empty used.
        let used_vec: Vec<WorkerId> = vec![];
        let used_set: BTreeSet<WorkerId> = BTreeSet::new();
        assert_eq!(
            elect_host_from_worker_names(&worker_names_with(pairs_by_id), &used_vec),
            elect_host_from_name_workers(&name_workers_with(pairs_by_name), &used_set),
        );
    }

    // --- Cycle-164b architect P3.1: sort-invariant debug-assert pin ---

    #[test]
    #[should_panic(expected = "must be strictly ascending")]
    #[cfg(debug_assertions)]
    fn backend_view_unsorted_used_panics_in_debug() {
        // Negative pin for the debug_assert added cycle 164b — proves
        // the guard bites when a future caller passes an unsorted
        // slice. In release builds the assert is compiled out, so
        // this test only runs in debug (cfg gate).
        let names = worker_names_with(&[(1, "w0"), (5, "host"), (9, "w1")]);
        let unsorted = vec![w(9), w(1), w(5)];
        let _ = elect_host_from_worker_names(&names, &unsorted);
    }
}
