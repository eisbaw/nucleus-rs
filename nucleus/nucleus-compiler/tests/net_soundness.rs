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
use nucleus_compiler::passes::net_soundness::{check_net_sound, PetriAnalysisError};
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
