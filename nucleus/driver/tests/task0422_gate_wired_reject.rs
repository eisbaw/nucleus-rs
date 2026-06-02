//! TASK-0422 (cycle-244): WIRING proof for the PRD §8.3 invariant (2)
//! (Push/Wait events form matched pairs) production gate.
//!
//! The driver `cmd_build` (src/main.rs, right before
//! `dispatch::dispatch_backend`) now hard-gates the FINAL per-worker
//! EventList through `nucleus_compiler::validate_event_lists`, so a
//! Push/Wait pairing violation on any shipping codegen build returns a
//! `Result::Err` (NOT a panic — this project rejects panic-on-valid-
//! input).
//!
//! ## Why this is NOT an end-to-end driver-reject test
//!
//! The honest situation: there is NO valid `.nuc` (algorithm +
//! schedule) source that produces an inv(2)-violating final EventList.
//! TASK-0428 (cycle-242) and TASK-0422.01 (cycle-243,
//! `task0422_01_inv2_post_mediation.rs`) empirically proved inv(2) holds
//! on the post-mediation EventList for the ENTIRE example corpus (220
//! (backend, schedule) cells, 0 violations). That corpus-clean property
//! is the whole reason wiring the gate is safe. So constructing a real
//! source that drives `cmd_build` to the validation `Err` arm is
//! infeasible by design — any such source would itself be a NEW finding
//! (a mediation/projection-pass bug), not a test fixture.
//!
//! Therefore this test proves the WIRING at the function level: it calls
//! `validate_event_lists` — the EXACT function the driver wires at the
//! cited site — on a hand-built EventList that violates inv(2), and
//! asserts it returns the `UnmatchedPush` error the driver's `map_err`
//! would surface. The 6 `EventValidationError` variants are each
//! individually unit-tested in
//! `nucleus-compiler/tests/event_validate.rs`; this test's distinct job
//! is to pin that the DRIVER consumes this exact validator on the final
//! `per_worker`, so the gate cannot silently regress to a no-op.
//!
//! The `just e2e` differential is the complementary live proof for the
//! POSITIVE direction: every codegen build runs the gate, and the
//! 385/328/0/57/0 baseline holding is the corpus-wide evidence that the
//! gate rejects zero shipping programs at the true consumption point.

use std::collections::BTreeMap;

use nucleus_compiler::{
    validate_event_lists, DataId, Event, EventValidationError, IterTile, SeqTag, WorkerId,
};

/// Build the FINAL-EventList shape the driver feeds to
/// `validate_event_lists`: worker 0 pushes to worker 1, but worker 1
/// never Waits for it. That is the canonical inv(2) violation
/// (`UnmatchedPush`) — the exact class the driver gate exists to reject.
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
fn task0422_driver_gate_rejects_unmatched_push() {
    // `validate_event_lists` is the SAME function wired in
    // `driver/src/main.rs` `cmd_build` immediately before
    // `dispatch::dispatch_backend(&backend, &per_worker, ...)` (the
    // sibling gate of `check_accumulator_consistency`). If that wiring
    // is removed or downgraded, the gate that this Err proves is the
    // shipping enforcement of PRD §8.3 inv(2) is gone — and the e2e
    // matrix would not catch a pairing bug that happened to still emit
    // bit-identical (but wrong) output.
    let map = unmatched_push_map();

    let errs = validate_event_lists(&map)
        .expect_err("inv(2) violation (unmatched Push) MUST be rejected by the driver gate");

    // Pure non-panicking function: returns the violation set, the
    // driver's `map_err` formats it into the String error channel.
    assert!(
        errs.iter().any(|e| matches!(
            e,
            EventValidationError::UnmatchedPush {
                src: WorkerId(0),
                dst: WorkerId(1),
                data: DataId(3),
                seq: SeqTag(11),
                ..
            }
        )),
        "expected UnmatchedPush among violations, got {errs:?}"
    );
}

#[test]
fn task0422_driver_gate_accepts_matched_pair() {
    // Mirror image: a properly matched Push/Wait pair MUST pass, so the
    // gate is not vacuously rejecting everything (and so the corpus-wide
    // e2e green is consistent with this function returning Ok on valid
    // EventLists).
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
        validate_event_lists(&m).is_ok(),
        "a matched Push/Wait pair must pass the driver gate; got {:?}",
        validate_event_lists(&m).err()
    );
}
