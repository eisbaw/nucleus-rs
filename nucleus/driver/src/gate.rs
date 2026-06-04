//! Post-projection EventList gates for `nucleus build`.
//!
//! Extracted from `main.rs` `cmd_build` (TASK-0422.02, cycle-245) so the
//! reject path of the final pre-dispatch gate is unit-testable. Before
//! this carve-out the gate was an inline pair of `?`-propagating calls a
//! refactor could silently delete: the unit bite test
//! (`driver/tests/task0422_gate_wired_reject.rs`) only exercised
//! `validate_event_lists` in ISOLATION, and `just e2e` is positive-
//! direction only (it proves the gate rejects no valid program, never
//! that the gate is still PRESENT/invoked — see this module's test
//! docstrings). Naming the gate fn turns the `cmd_build` call into a
//! single visible line and makes the reject arm drivable from a test.
//!
//! WITNESS-CLOSED RESIDUAL (TASK-0440): the TASK-0422.02 extraction
//! proved the gate COMPOSITION rejects an inv(2)-violating EventList and
//! reduced the `cmd_build`-side risk to one greppable call line, but it
//! could NOT prove that literal call line in `cmd_build` still EXECUTES
//! — a refactor that deleted `gate::gate_per_worker_for_dispatch(...)?`
//! would still pass every test (unit + e2e). TASK-0422.02 recorded this
//! as an "honest residual" and claimed the ONLY way to close it was a
//! `#[cfg(test)]`-gated fault-injection hook in the production hot path,
//! deliberately declined on maintainability grounds.
//!
//! That claim was INCOMPLETE. The residual is now CLOSED by a
//! COMPILE-TIME WITNESS ([`GatedPerWorker`], TASK-0440): the gate fn
//! returns a token whose ONLY constructor is the gate's own success
//! path, and `dispatch::dispatch_backend` consumes that token INSTEAD of
//! a bare `&per_worker`. Deleting or bypassing the `cmd_build` gate call
//! therefore leaves no token to dispatch with — the driver FAILS TO
//! COMPILE. This is strictly stronger than a runtime fault hook (it
//! cannot regress even if every test were deleted) and adds ZERO
//! production-hot-path test scaffolding, resolving the exact
//! maintainability objection TASK-0422.02 raised. The runtime gate LOGIC
//! (that the validator actually rejects) stays proven by the unit tests
//! below + `nucleus-compiler/tests/event_validate.rs`.

use std::collections::BTreeMap;

use nucleus_compiler::algo::AlgoIR;
use nucleus_compiler::{validate_event_lists, DataId, Event, NameSidecar, WorkerId};

/// Compile-time witness that the post-projection EventList gate
/// ([`gate_per_worker_for_dispatch`]) RAN and ACCEPTED `per_worker`
/// (TASK-0440).
///
/// The `per_worker` field is PRIVATE and there is no `pub` constructor,
/// so no module outside `gate` can build one. The ONLY value of this
/// type comes from the gate fn's success path — therefore *holding* a
/// `GatedPerWorker` is PROOF the gate ran on that exact EventList.
///
/// `dispatch::dispatch_backend` takes this token instead of a bare
/// `&BTreeMap<WorkerId, Vec<Event>>`, which makes the `cmd_build` gate
/// call COMPILE-TIME LOAD-BEARING: deleting it leaves no token to
/// dispatch with, so the driver fails to build. This SUPERSEDES the
/// TASK-0422.02 "honest residual" (which declined a runtime
/// fault-injection hook on maintainability grounds) with a stronger,
/// zero-runtime-cost mechanism.
///
/// `Debug` is derived only so `Result::expect_err` (used by the reject
/// unit test) can format the `Ok` arm; it is not relied on otherwise.
#[derive(Debug)]
pub(crate) struct GatedPerWorker<'a> {
    per_worker: &'a BTreeMap<WorkerId, Vec<Event>>,
}

impl<'a> GatedPerWorker<'a> {
    /// The gated EventList this token witnesses. Borrows for `'a`, the
    /// same lifetime as the `per_worker` the gate accepted.
    pub(crate) fn events(&self) -> &'a BTreeMap<WorkerId, Vec<Event>> {
        self.per_worker
    }
}

/// The post-projection EventList gates EVERY backend's input must pass
/// before `dispatch::dispatch_backend`. Gated ONCE here on the FINAL
/// `per_worker` (after projection + `inject_check_frames` + the mp-*
/// `safe_push_reorder` / `host_mediation_inject` / `host_data_relay_inject`
/// re-routing), so ONE site covers all 7 backends.
///
/// Two checks, in this order (order is load-bearing — accumulator THEN
/// validate, matching the historical inline sequence):
///
/// 1. **Overlapping-write accumulator algorithm-level cross-check**
///    (TASK-0343.03; hardens the cycle-189 structural detector
///    TASK-0343). The backends classify the overlapping-write
///    accumulator fan-in PURELY STRUCTURALLY (per worker, >=2 whole-array
///    Waits on one data symbol => element-wise sum combine at the host —
///    `collect_accumulate_waits`). For every shipped schedule that
///    structural shape coincides with the algorithm-level accumulator
///    shape (LHS-appears-in-RHS, e.g. 08-histogram's
///    `histogram[b] <-- bin_inc(histogram[b], ...)`), so this gate is a
///    NO-OP on the entire e2e matrix. It exists to FAIL LOUD if an exotic
///    schedule ever emits multiple whole-array pushes for NON-accumulator
///    semantics, which the structural detector would otherwise silently
///    mis-combine as a sum (a silent miscompile). It reuses the EXACT
///    structural detector the backends consume (no duplicated detection
///    logic) and consults `algo` for the LHS-appears-in-RHS shape via
///    `data_names` (DataId -> name) as the bridge between the codegen
///    DataId space and the algorithm-IR String-name space. The detector
///    is order-insensitive, so the result is backend-independent even
///    though the mp-* `per_worker` is the `safe_push_reorder`-transformed
///    map.
///
/// 2. **PRD §8.3 event-contract validation** (`validate_event_lists`;
///    TASK-0422 wired the call, TASK-0423 widened it). This enforces the
///    full event-contract surface — invariants 1, 2, 3 AND 6 (Sync
///    participant agreement); invariants 4/5 (Alloc/Free) stay latent.
///    The frequently-cited motivating case is inv(2): Push/Wait events
///    form matched pairs. Both post-projection transforms PRESERVE the
///    Push/Wait set, so validating here is equivalent to validating at
///    the projection boundary, but at the TRUE backend-consumption point
///    and AFTER the mp-* re-routing.
///
///    Why a hard `Result<(), String>` gate and NOT a `panic!`: this
///    project rejects panic-on-valid-input (memory
///    `feedback-panic-not-diagnostic-recurring`). `validate_event_lists`
///    is a pure, non-panicking function returning the full set of
///    violations with a deterministic `Debug`.
///
///    Why this is SAFE to wire as a hard gate (returns Ok on every
///    shipping program): TASK-0428 (cycle-242) proved inv(2) on the
///    backend-agnostic pre-mediation EventList for the entire example
///    corpus, and TASK-0422.01 (cycle-243,
///    `driver/tests/task0422_01_inv2_post_mediation.rs`) proved it on the
///    POST-mediation EventList for all 4 mp-* backends (220 cells, 0
///    violations). The `just e2e` differential is the corpus-wide live
///    proof it rejects ZERO shipping programs at the true consumption
///    point. It exists to FAIL LOUD if a future pass regression breaks
///    the contract.
///
/// NOTE: the `acfg_to_events` `debug_assert!` (`petri_to_events.rs`)
/// deliberately stays the strict-per-worker SUBSET (excludes inv(2))
/// because that boundary is ALSO hit by the driver's host-election
/// preview projections on pre-mediation ACFGs, where inv(2) need not yet
/// hold. The FULL validator belongs ONLY here, at the final consumption
/// point.
///
/// Both checks return their error via the String channel (`?` in
/// `cmd_build` -> process exit 1, no panic). The error strings are
/// byte-preserved from the historical inline sequence so behavior / e2e
/// is unchanged by the extraction.
///
/// On success returns a [`GatedPerWorker`] witness (TASK-0440) that
/// borrows the accepted `per_worker`. That token is the SOLE input
/// `dispatch::dispatch_backend` accepts, so a caller cannot dispatch
/// without first calling this gate — the wiring is enforced by the type
/// system, not just by convention.
pub(crate) fn gate_per_worker_for_dispatch<'a>(
    algo: &AlgoIR,
    per_worker: &'a BTreeMap<WorkerId, Vec<Event>>,
    sidecar: &NameSidecar,
    data_names: &BTreeMap<DataId, String>,
) -> Result<GatedPerWorker<'a>, String> {
    backend_common::multi_worker_walker::check_accumulator_consistency(
        algo,
        per_worker,
        sidecar,
        data_names,
    )
    .map_err(|e| format!("accumulator cross-check error: {e}"))?;

    validate_event_lists(per_worker).map_err(|errs| {
        format!(
            "event-contract validation failed (PRD §8.3 inv(2), Push/Wait pairing): \
             {} violation(s): {errs:?}",
            errs.len()
        )
    })?;

    Ok(GatedPerWorker { per_worker })
}

#[cfg(test)]
mod tests {
    //! TASK-0422.02 (cycle-245): prove the DRIVER's post-projection gate
    //! COMPOSITION rejects an inv(2)-violating EventList — not just the
    //! `validate_event_lists` validator in isolation
    //! (`driver/tests/task0422_gate_wired_reject.rs` already covers that).
    //!
    //! These tests exercise `gate_per_worker_for_dispatch`, the EXACT fn
    //! `cmd_build` calls immediately before `dispatch::dispatch_backend`.
    //! Because no valid OR invalid `.nuc` source produces an inv(2)
    //! violation on the FINAL `per_worker` (TASK-0428 / TASK-0422.01
    //! proved the corpus is inv(2)-clean), the reject arm is undriveable
    //! from real input — so this hand-built map is the only way to bite
    //! it. We do NOT fake an end-to-end `.nuc` reject (none exists).
    //!
    //! These prove the extracted gate fn rejects (runtime LOGIC). That
    //! the literal `cmd_build` call line executes is now proven SEPARATELY
    //! and more strongly by the [`GatedPerWorker`] compile-time witness
    //! (TASK-0440, see the module docstring): deleting the call fails to
    //! build. So these tests own the runtime-detection proof, the witness
    //! owns the wired-presence proof — together they close the residual
    //! TASK-0422.02 left open. The accumulator cross-check arm is a no-op
    //! on this input (no whole-array multi-Wait), so the rejection comes
    //! from the `validate_event_lists` arm — which is exactly the gate
    //! TASK-0422 wired.

    use super::*;
    use nucleus_compiler::{IterTile, SeqTag};

    /// Build the FINAL-EventList shape `cmd_build` feeds to the gate:
    /// worker 0 pushes to worker 1, but worker 1 never Waits for it. That
    /// is the canonical inv(2) violation (`UnmatchedPush`) — the exact
    /// class the gate exists to reject. (Construction mirrors
    /// `driver/tests/task0422_gate_wired_reject.rs`.)
    fn unmatched_push_map() -> BTreeMap<WorkerId, Vec<Event>> {
        let mut m = BTreeMap::new();
        m.insert(
            WorkerId(0),
            vec![Event::Push {
                dst: WorkerId(1),
                data: DataId(3),
                tile: IterTile::empty(),
                seq: SeqTag(11),
            }],
        );
        // Worker 1 declared but never Waits for the push above.
        m.insert(WorkerId(1), vec![]);
        m
    }

    #[test]
    fn gate_rejects_unmatched_push() {
        // Minimal valid sibling args: an EMPTY algo has no accumulator
        // names, and the map has no whole-array multi-Wait, so the
        // accumulator cross-check is a no-op — the rejection is produced
        // by the `validate_event_lists` arm, which is the gate under
        // test. `data_names` empty for the same reason (cross-check never
        // looks anything up).
        let algo = AlgoIR::default();
        let sidecar = NameSidecar::default();
        let data_names: BTreeMap<DataId, String> = BTreeMap::new();

        let err = gate_per_worker_for_dispatch(
            &algo,
            &unmatched_push_map(),
            &sidecar,
            &data_names,
        )
        .expect_err("inv(2) violation (unmatched Push) MUST be rejected by the driver gate");

        // Pin the event-contract-validation error text (not the
        // accumulator arm), so a regression that re-orders or drops the
        // validate arm is caught.
        assert!(
            err.contains("event-contract validation failed (PRD §8.3 inv(2), Push/Wait pairing)"),
            "expected event-contract validation error from the gate, got: {err}"
        );
        assert!(
            err.contains("violation(s)"),
            "expected the violation-count formatting from the gate, got: {err}"
        );
    }

    #[test]
    fn gate_accepts_matched_pair() {
        // Non-tautology guard: a properly matched Push/Wait pair MUST
        // pass the gate composition, so the reject above is meaningful
        // (and consistent with corpus-wide e2e green).
        let algo = AlgoIR::default();
        let sidecar = NameSidecar::default();
        let data_names: BTreeMap<DataId, String> = BTreeMap::new();

        let mut m = BTreeMap::new();
        m.insert(
            WorkerId(0),
            vec![Event::Push {
                dst: WorkerId(1),
                data: DataId(3),
                tile: IterTile::empty(),
                seq: SeqTag(11),
            }],
        );
        m.insert(
            WorkerId(1),
            vec![Event::Wait {
                src: WorkerId(0),
                data: DataId(3),
                tile: IterTile::empty(),
                seq: SeqTag(11),
            }],
        );

        assert!(
            gate_per_worker_for_dispatch(&algo, &m, &sidecar, &data_names).is_ok(),
            "a matched Push/Wait pair must pass the gate composition; got {:?}",
            gate_per_worker_for_dispatch(&algo, &m, &sidecar, &data_names).err()
        );
    }

    #[test]
    fn gate_witness_threads_event_list_through() {
        // The GatedPerWorker witness (TASK-0440) must hand back the SAME
        // EventList the gate accepted — `dispatch_backend` reads its
        // `per_worker` exclusively through `gated.events()`, so a witness
        // that returned a different/empty map would silently feed the
        // backends the wrong contract. Pin pointer identity (same `&map`).
        let algo = AlgoIR::default();
        let sidecar = NameSidecar::default();
        let data_names: BTreeMap<DataId, String> = BTreeMap::new();

        let mut m = BTreeMap::new();
        m.insert(
            WorkerId(0),
            vec![Event::Push {
                dst: WorkerId(1),
                data: DataId(3),
                tile: IterTile::empty(),
                seq: SeqTag(11),
            }],
        );
        m.insert(
            WorkerId(1),
            vec![Event::Wait {
                src: WorkerId(0),
                data: DataId(3),
                tile: IterTile::empty(),
                seq: SeqTag(11),
            }],
        );

        let gated = gate_per_worker_for_dispatch(&algo, &m, &sidecar, &data_names)
            .expect("matched Push/Wait pair must pass the gate");
        // Same logical contents...
        assert_eq!(gated.events(), &m);
        // ...and the same borrow (no copy/rebuild of the map).
        assert!(
            std::ptr::eq(gated.events(), &m),
            "witness must borrow the gated map, not a rebuilt copy"
        );
    }
}
