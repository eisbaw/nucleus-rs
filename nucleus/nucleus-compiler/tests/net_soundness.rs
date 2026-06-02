//! Integration tests for the combined Petri-net soundness gate
//! (TASK-0368, PRD §8.1 / §8.4).
//!
//! The gate `check_net_sound` is what the production driver
//! (`nucleus/driver/src/main.rs`) runs on EVERY build: a net that is
//! unbounded (capacity overflow) or deadlocking (a stall in the
//! derived firing order) is rejected as a compile error.
//!
//! ## Why these are synthetic, hand-built nets
//!
//! We canNOT drive an unsound net through the full driver: no valid
//! schedule produces one. The structural inject-pass guards
//! (TtoP-arc elision + ad-hoc ACFG guards in sync_inject /
//! transfer_inject) mean every net the shipping pipeline emits is
//! bounded and deadlock-free by construction — empirically verified
//! over all examples x schedules x 7 tier-1 backends (TASK-0368). The
//! gate is therefore a *provably-dead-today tripwire* on shipping
//! schedules.
//!
//! These tests pin the gate's reject path at the function level by
//! feeding it hand-built unsound nets (templated from the synthetic
//! cases in `tests/boundedness.rs` and `tests/deadlock.rs`). Their job
//! is to guard against a FUTURE regression in the inject passes — one
//! that DID emit an unsound net — so it would surface as a compile
//! error rather than shipping as a runtime hang or buffer overrun.
//!
//! ## Positive coverage lives elsewhere
//!
//! `check_net_sound` composes `check_bounded` + `check_deadlock_free`,
//! whose end-to-end positive cases (real examples pass both) are
//! covered by `tests/boundedness.rs` and `tests/deadlock.rs`. The
//! full e2e build matrix (`just e2e`) is the integration-level proof
//! that the gate rejects nothing shipping. Here we add one positive
//! smoke (a matched producer/consumer net passes) plus the two
//! reject paths.

use std::num::NonZeroU32;

use nucleus_compiler::passes::boundedness::BoundednessError;
use nucleus_compiler::passes::deadlock::DeadlockError;
use nucleus_compiler::passes::net_soundness::{
    check_conflict_free, check_net_sound, ConflictError, PetriAnalysisError,
};
use nucleus_compiler::petri::{ArcKind, Net};

fn cap(n: u32) -> Option<NonZeroU32> {
    Some(NonZeroU32::new(n).expect("test caps are >0"))
}

// --------------------------------------------------------------------
// Reject path 1: unbounded net (capacity overflow)
// --------------------------------------------------------------------

/// A 2-token push into a capacity-1 place. Templated from
/// `tests/boundedness.rs::two_token_push_into_cap1_place_is_rejected`.
/// `check_net_sound` must reject with `PetriAnalysisError::Boundedness`
/// (boundedness runs before deadlock, and this net overflows on its
/// only firing).
#[test]
fn unbounded_net_rejected_as_boundedness_error() {
    let mut net = Net::new();
    let src = net.add_place("source", cap(1), 1);
    let buf = net.add_place("buf", cap(1), 0);
    let t = net.add_transition("overflower", None);
    net.add_arc(ArcKind::PtoT, src, t, 1);
    // Weight-2 arc — the offending production into a cap-1 place.
    net.add_arc(ArcKind::TtoP, buf, t, 2);

    let err = check_net_sound(&net).expect_err("unbounded net must be rejected");
    match err {
        PetriAnalysisError::Boundedness(BoundednessError::CapacityExceeded {
            place_name,
            transition_name,
            would_be,
            capacity,
            ..
        }) => {
            assert_eq!(place_name, "buf");
            assert_eq!(transition_name, "overflower");
            assert_eq!(would_be, 2);
            assert_eq!(capacity.get(), 1);
        }
        other => panic!("expected Boundedness(CapacityExceeded), got {other:?}"),
    }
}

// --------------------------------------------------------------------
// Reject path 2: deadlocking net (unmatched wait)
// --------------------------------------------------------------------

/// A Wait transition with no matching Push — the missing-producer
/// shape. Templated from
/// `tests/deadlock.rs::unmatched_wait_is_detected_as_deadlock`.
///
/// Mechanism (important — this is the gate's semantic-labelling path,
/// not a clean "passes bounded, fails deadlock"): the single derived
/// firing order is `[wait_seq0]`, and `wait_seq0` cannot fire (its
/// `buf_seq0` input is empty). `check_bounded` (which the gate runs
/// FIRST) therefore returns `BoundednessError::InvalidFiringOrder` —
/// not `Ok`. Because a stall is "deadlock-territory, not boundedness"
/// (per that variant's own docstring), `check_net_sound` does NOT
/// label it `Boundedness`; it falls through to `check_deadlock_free`,
/// which diagnoses the identical stall precisely as
/// `DeadlockError::Stalled`. So the gate must reject with
/// `PetriAnalysisError::Deadlock(Stalled)`. This asserts the
/// fall-through, which is the part of the gate most likely to regress.
#[test]
fn deadlocking_net_rejected_as_deadlock_error() {
    let mut net = Net::new();
    let buf = net.add_place("buf_seq0", cap(1), 0);
    let ctl = net.add_place("ctl_w_0", cap(1), 1);
    let ctl_next = net.add_place("ctl_w_1", cap(1), 0);

    let wait = net.add_transition("wait_seq0", None);
    net.add_arc(ArcKind::PtoT, ctl, wait, 1);
    net.add_arc(ArcKind::PtoT, buf, wait, 1);
    net.add_arc(ArcKind::TtoP, ctl_next, wait, 1);

    let err = check_net_sound(&net).expect_err("deadlocking net must be rejected");
    match err {
        PetriAnalysisError::Deadlock(DeadlockError::Stalled {
            transition_name,
            place_name,
            have,
            need,
            ..
        }) => {
            assert_eq!(transition_name, "wait_seq0");
            assert_eq!(place_name, "buf_seq0");
            assert_eq!(have, 0);
            assert_eq!(need, 1);
        }
        other => panic!("expected Deadlock(Stalled), got {other:?}"),
    }
}

// --------------------------------------------------------------------
// Positive smoke: a sound net passes the gate
// --------------------------------------------------------------------

/// A matched producer/consumer net with a capacity-1 buffer and
/// per-worker control chains that force strict alternation. Both
/// analyses must pass, so `check_net_sound` returns `Ok(())`. This is
/// the function-level mirror of "the gate rejects nothing shipping"
/// (the full e2e matrix is the integration-level proof).
#[test]
fn sound_matched_producer_consumer_passes() {
    let mut net = Net::new();
    let ctl_p0 = net.add_place("ctl_p_0", cap(1), 1);
    let ctl_p1 = net.add_place("ctl_p_1", cap(1), 0);
    let ctl_p2 = net.add_place("ctl_p_2", cap(1), 0);
    let ctl_c0 = net.add_place("ctl_c_0", cap(1), 1);
    let ctl_c1 = net.add_place("ctl_c_1", cap(1), 0);
    let ctl_c2 = net.add_place("ctl_c_2", cap(1), 0);
    let buf = net.add_place("buf", cap(1), 0);

    let produce0 = net.add_transition("produce0", None);
    net.add_arc(ArcKind::PtoT, ctl_p0, produce0, 1);
    net.add_arc(ArcKind::TtoP, ctl_p1, produce0, 1);
    net.add_arc(ArcKind::TtoP, buf, produce0, 1);

    let consume0 = net.add_transition("consume0", None);
    net.add_arc(ArcKind::PtoT, ctl_c0, consume0, 1);
    net.add_arc(ArcKind::TtoP, ctl_c1, consume0, 1);
    net.add_arc(ArcKind::PtoT, buf, consume0, 1);

    let produce1 = net.add_transition("produce1", None);
    net.add_arc(ArcKind::PtoT, ctl_p1, produce1, 1);
    net.add_arc(ArcKind::TtoP, ctl_p2, produce1, 1);
    net.add_arc(ArcKind::TtoP, buf, produce1, 1);

    let consume1 = net.add_transition("consume1", None);
    net.add_arc(ArcKind::PtoT, ctl_c1, consume1, 1);
    net.add_arc(ArcKind::TtoP, ctl_c2, consume1, 1);
    net.add_arc(ArcKind::PtoT, buf, consume1, 1);

    // `derive_firing_order` walks marking-aware firable transitions in
    // source order; this net has a legal interleaving the gate finds.
    check_net_sound(&net).expect("matched producer/consumer net must be sound");
}

// --------------------------------------------------------------------
// Reject path 3: free-choice conflict (PRD §8.4(a), TASK-0421)
// --------------------------------------------------------------------

/// A genuine free-choice conflict: ONE place holding a token, consumed
/// by TWO distinct transitions that are BOTH enabled at the initial
/// marking. Which one fires is decided by run-time token availability,
/// not by the static order — exactly the §8.4(a) violation the new
/// `check_conflict_free` gate catches.
///
/// Net shape (deliberately NOT control-place-threaded, so nothing
/// serialises the two consumers — this is what a regressed inject pass
/// could emit but `acfg_to_net` cannot today):
///
/// ```text
///   (shared:1) --PtoT--> [drain_a] --TtoP--> (sink_a:0/cap1)
///        \------PtoT----> [drain_b] --TtoP--> (sink_b:0/cap1)
/// ```
///
/// At the initial marking `shared` holds 1 token, so BOTH `drain_a`
/// and `drain_b` are consumption-enabled (each needs 1) — the conflict.
/// `check_net_sound` must reject with
/// `PetriAnalysisError::ConflictingChoice` naming `shared` and both
/// drains, and (because the conflict arm runs FIRST) BEFORE any
/// boundedness/deadlock answer is computed.
///
/// PROVE-IT-BITES: this rejection comes from the new conflict arm. The
/// companion test `free_choice_net_is_flagged_by_conflict_pass_directly`
/// pins that `check_conflict_free` alone flags this exact net, so
/// removing the `check_conflict_free(...)?` line from `check_net_sound`
/// changes the rejection (the net no longer fails for the right
/// reason). The conflict arm is therefore load-bearing.
#[test]
fn free_choice_conflict_net_rejected() {
    let mut net = Net::new();
    let shared = net.add_place("shared", cap(1), 1);
    let sink_a = net.add_place("sink_a", cap(1), 0);
    let sink_b = net.add_place("sink_b", cap(1), 0);

    let drain_a = net.add_transition("drain_a", None);
    net.add_arc(ArcKind::PtoT, shared, drain_a, 1);
    net.add_arc(ArcKind::TtoP, sink_a, drain_a, 1);

    let drain_b = net.add_transition("drain_b", None);
    net.add_arc(ArcKind::PtoT, shared, drain_b, 1);
    net.add_arc(ArcKind::TtoP, sink_b, drain_b, 1);

    let err = check_net_sound(&net).expect_err("free-choice net must be rejected");
    match err {
        PetriAnalysisError::ConflictingChoice(ConflictError::FreeChoice {
            place_name,
            transitions,
            position,
            ..
        }) => {
            assert_eq!(place_name, "shared");
            // Both drains, sorted by TransitionId (drain_a id 0 < drain_b id 1).
            let names: Vec<&str> = transitions.iter().map(|(_, n)| n.as_str()).collect();
            assert_eq!(names, vec!["drain_a", "drain_b"]);
            // Conflict is visible at the initial marking.
            assert_eq!(position, 0);
        }
        other => panic!("expected ConflictingChoice(FreeChoice), got {other:?}"),
    }
}

/// PROVE-IT-BITES companion. Asserts `check_conflict_free` run ALONE
/// flags this exact free-choice net (the conflict arm detects the
/// defect on its own, so it is load-bearing in `check_net_sound`).
///
/// HONEST NOTE on what removing the arm actually does (empirically
/// verified, TASK-0421): this net does NOT slip silently through the
/// gate without the conflict arm — `derive_firing_order` fires `drain_a`
/// (source-order-first), empties `shared`, then appends the now-unfirable
/// `drain_b`; the deadlock pass stalls on it and `check_net_sound` would
/// return `Deadlock(Stalled { place: "shared", position: 1 })`. So the
/// arm's value is the SEMANTICALLY CORRECT label (a nondeterministic /
/// free-choice net, reported as such at the initial marking) rather than
/// the misleading "deadlock at position 1" a downstream pass produces on
/// the one order it happened to pick. The sibling test
/// `free_choice_conflict_net_rejected` is what bites: with the conflict
/// arm removed it observes `Deadlock(Stalled ...)` instead of the
/// expected `ConflictingChoice` and FAILS.
#[test]
fn free_choice_net_is_flagged_by_conflict_pass_directly() {
    let mut net = Net::new();
    let shared = net.add_place("shared", cap(1), 1);
    let sink_a = net.add_place("sink_a", cap(1), 0);
    let sink_b = net.add_place("sink_b", cap(1), 0);

    let drain_a = net.add_transition("drain_a", None);
    net.add_arc(ArcKind::PtoT, shared, drain_a, 1);
    net.add_arc(ArcKind::TtoP, sink_a, drain_a, 1);

    let drain_b = net.add_transition("drain_b", None);
    net.add_arc(ArcKind::PtoT, shared, drain_b, 1);
    net.add_arc(ArcKind::TtoP, sink_b, drain_b, 1);

    // The conflict pass, run alone, rejects: at the initial marking two
    // consumers of `shared` are co-enabled.
    let err = check_conflict_free(&net).expect_err("conflict pass must flag the free-choice net");
    assert!(
        matches!(err, ConflictError::FreeChoice { .. }),
        "expected FreeChoice, got {err:?}"
    );
}

// --------------------------------------------------------------------
// Positive (no-false-reject): a shipping-shaped unrolled-loop net
// --------------------------------------------------------------------

/// A net mirroring the SHIPPING shape AC#1 found in the corpus: one
/// buffer place keyed by a single `SeqTag`, fed by two `Push`
/// transitions and consumed by two `Wait` transitions (the unrolled
/// copies of an in-loop transfer). The two waits each have PtoT arcs to
/// the SAME buffer place, so a NAIVE structural predicate ("place with
/// `>= 2` distinct consumers = conflict") would FALSE-REJECT this net —
/// which is exactly the panic-on-valid-input failure AC#1 ruled out.
///
/// They are NOT a conflict, because each wait is gated behind its own
/// per-worker control-place predecessor (exactly what `acfg_to_petri`'s
/// `thread_through_worker` emits): at any reachable marking at most ONE
/// wait is enabled, so the order is statically forced, not chosen. The
/// order-aware `check_conflict_free` must therefore ACCEPT it.
///
/// Net shape (consumer worker `c`; the two waits w0, w1 chained):
///
/// ```text
///   producer:  (ctl_p0:1) -[push0]-> (ctl_p1)            buf<-+1
///              (ctl_p1)   -[push1]-> (ctl_p2)            buf<-+1
///   consumer:  (ctl_c0:1) -[wait0]-> (ctl_c1)   buf -1->
///              (ctl_c1)   -[wait1]-> (ctl_c2)   buf -1->
/// ```
///
/// `buf` (capacity 2) has PtoT arcs to BOTH wait0 and wait1 — the naive
/// multi-out shape — yet wait1 cannot fire until wait0 has advanced the
/// consumer control chain to `ctl_c1`, so the two are never co-enabled.
#[test]
fn shipping_shaped_unrolled_loop_buffer_passes_no_false_reject() {
    let mut net = Net::new();

    // Producer control chain + the shared buffer place.
    let ctl_p0 = net.add_place("ctl_p_0", cap(1), 1);
    let ctl_p1 = net.add_place("ctl_p_1", cap(1), 0);
    let ctl_p2 = net.add_place("ctl_p_2", cap(1), 0);
    // One buffer place for the seq, capacity 2 (room for both pushes).
    let buf = net.add_place("buf_stream_seq0", cap(2), 0);

    // Consumer control chain.
    let ctl_c0 = net.add_place("ctl_c_0", cap(1), 1);
    let ctl_c1 = net.add_place("ctl_c_1", cap(1), 0);
    let ctl_c2 = net.add_place("ctl_c_2", cap(1), 0);

    // push0, push1 — deposit into buf, advance producer chain.
    let push0 = net.add_transition("push_seq0_iter0", None);
    net.add_arc(ArcKind::PtoT, ctl_p0, push0, 1);
    net.add_arc(ArcKind::TtoP, ctl_p1, push0, 1);
    net.add_arc(ArcKind::TtoP, buf, push0, 1);

    let push1 = net.add_transition("push_seq0_iter1", None);
    net.add_arc(ArcKind::PtoT, ctl_p1, push1, 1);
    net.add_arc(ArcKind::TtoP, ctl_p2, push1, 1);
    net.add_arc(ArcKind::TtoP, buf, push1, 1);

    // wait0, wait1 — consume from buf (BOTH PtoT to `buf`), advance
    // consumer chain. This is the multi-out place a naive check trips on.
    let wait0 = net.add_transition("wait_seq0_iter0", None);
    net.add_arc(ArcKind::PtoT, ctl_c0, wait0, 1);
    net.add_arc(ArcKind::TtoP, ctl_c1, wait0, 1);
    net.add_arc(ArcKind::PtoT, buf, wait0, 1);

    let wait1 = net.add_transition("wait_seq0_iter1", None);
    net.add_arc(ArcKind::PtoT, ctl_c1, wait1, 1);
    net.add_arc(ArcKind::TtoP, ctl_c2, wait1, 1);
    net.add_arc(ArcKind::PtoT, buf, wait1, 1);

    // Sanity: `buf` really is a naive multi-out place (>= 2 distinct
    // PtoT consumers), so this test exercises the false-reject path.
    let buf_consumers = net
        .arcs
        .iter()
        .filter(|a| a.kind == ArcKind::PtoT && a.place == buf)
        .map(|a| a.transition)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        buf_consumers.len(),
        2,
        "test fixture must have buf consumed by 2 distinct transitions"
    );

    // The order-aware gate must accept it (no false-reject).
    check_conflict_free(&net)
        .expect("shipping-shaped unrolled-loop buffer must NOT be flagged as a conflict");
    check_net_sound(&net)
        .expect("shipping-shaped unrolled-loop buffer net must pass the full gate");
}

// --------------------------------------------------------------------
// Determinism: the gate is a pure function of the net
// --------------------------------------------------------------------

/// `check_net_sound` is built from deterministic passes
/// (`derive_firing_order` + BTreeMap-backed replay), so the same net
/// yields the same result — including the same typed error payload —
/// on repeated calls. Pin it on the reject path (the payload is the
/// part most likely to drift).
#[test]
fn check_net_sound_is_deterministic_on_reject() {
    let mut net = Net::new();
    let src = net.add_place("source", cap(1), 1);
    let buf = net.add_place("buf", cap(1), 0);
    let t = net.add_transition("overflower", None);
    net.add_arc(ArcKind::PtoT, src, t, 1);
    net.add_arc(ArcKind::TtoP, buf, t, 2);

    let e1 = check_net_sound(&net).expect_err("rejects");
    let e2 = check_net_sound(&net).expect_err("rejects");
    assert_eq!(e1, e2, "check_net_sound must be deterministic");
}
