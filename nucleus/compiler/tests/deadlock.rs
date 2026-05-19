//! Integration tests for the deadlock analysis pass
//! (TASK-0029, PRD §8.4).
//!
//! Strategy
//! --------
//!
//! - **Synthetic negative case**: a Wait transition with no matching
//!   Push (the missing-producer shape). `check_deadlock_free` must
//!   reject and the error must name the stalled transition and the
//!   deficit place.
//!
//! - **Synthetic positive case**: a small producer/consumer chain
//!   wired up correctly. `check_deadlock_free` must accept.
//!
//! - **End-to-end on example 02 split**: this is the pass paying its
//!   keep. transfer_inject (TASK-0136/0139) is known to splice Waits
//!   without their matching Pushes in some configurations. The
//!   resulting net deadlocks at the first `wait_seq*` transition.
//!   We document this as a known upstream bug and assert that the
//!   deadlock pass detects it.
//!
//! - **End-to-end on example 01 naive**: single worker, no
//!   cross-worker transfers. Must be deadlock-free.
//!
//! - **Determinism**: same input yields the same output, including
//!   the same error payload (marking snapshot, position).

use std::num::NonZeroU32;

use compiler::algo::{lower_algo, parse_algo};
use compiler::link;
use compiler::passes::acfg_to_petri::acfg_to_net;
use compiler::passes::boundedness::derive_firing_order;
use compiler::passes::deadlock::{check_deadlock_free, DeadlockError};
use compiler::passes::sync_inject::inject_syncs;
use compiler::passes::transfer_inject::inject_transfers;
use compiler::petri::{ArcKind, Net, TransitionId};
use compiler::sched::{lower_sched, parse_sched};

// --------------------------------------------------------------------
// Helpers
// --------------------------------------------------------------------

fn cap(n: u32) -> Option<NonZeroU32> {
    Some(NonZeroU32::new(n).expect("test caps are >0"))
}

fn read_example(relpath: &str) -> String {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let repo_root = std::path::Path::new(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let full = repo_root.join("nuc-nucleus").join("examples").join(relpath);
    std::fs::read_to_string(&full)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", full.display(), e))
}

fn pipeline_to_net(algo_rel: &str, sched_rel: &str) -> Net {
    let algo_ast = parse_algo(&read_example(algo_rel)).expect("algo parse");
    let algo = lower_algo(&algo_ast).expect("algo lower");
    let sched_ast = parse_sched(&read_example(sched_rel)).expect("sched parse");
    let sched = lower_sched(&sched_ast).expect("sched lower");
    let linked = link::link(algo, sched).expect("link");
    let acfg = compiler::acfg::build_acfg(&linked).expect("build_acfg");
    let acfg = inject_syncs(acfg);
    let acfg = inject_transfers(&linked, acfg);
    acfg_to_net(&acfg)
}

// --------------------------------------------------------------------
// Synthetic negative case: Wait with no matching Push
// --------------------------------------------------------------------

/// Build a tiny net mirroring the "Wait with no matching Push" shape:
///   - a buffer place `buf` (capacity 1), starts empty,
///   - one transition `wait_seq0` that consumes a token from `buf`,
///   - **no producer** — nothing ever deposits into `buf`.
///
/// Firing `wait_seq0` first would stall (buf has 0 tokens, needs 1).
/// This mimics the situation where transfer_inject splices a Wait
/// without its peer Push.
#[test]
fn unmatched_wait_is_detected_as_deadlock() {
    let mut net = Net::new();
    let buf = net.add_place("buf_seq0", cap(1), 0);
    let ctl = net.add_place("ctl_w_0", cap(1), 1);
    let ctl_next = net.add_place("ctl_w_1", cap(1), 0);

    let wait = net.add_transition("wait_seq0", None);
    net.add_arc(ArcKind::PtoT, ctl, wait, 1);
    net.add_arc(ArcKind::PtoT, buf, wait, 1);
    net.add_arc(ArcKind::TtoP, ctl_next, wait, 1);

    let err = check_deadlock_free(&net, &[wait]).expect_err("must detect stall");
    match err {
        DeadlockError::Stalled {
            transition,
            transition_name,
            place,
            place_name,
            have,
            need,
            position,
            ..
        } => {
            assert_eq!(transition, wait);
            assert_eq!(transition_name, "wait_seq0");
            assert_eq!(place, buf);
            assert_eq!(place_name, "buf_seq0");
            assert_eq!(have, 0);
            assert_eq!(need, 1);
            assert_eq!(position, 0);
        }
        other => panic!("expected Stalled, got {:?}", other),
    }
}

// --------------------------------------------------------------------
// Synthetic positive case: matched producer/consumer chain
// --------------------------------------------------------------------

/// Two workers' worth of per-worker control plus a single
/// produce→consume transfer through a capacity-1 buffer. Firing
/// order produces first, then consumes. `check_deadlock_free` must
/// accept.
#[test]
fn matched_producer_consumer_chain_is_deadlock_free() {
    let mut net = Net::new();
    let ctl_p0 = net.add_place("ctl_p_0", cap(1), 1);
    let ctl_p1 = net.add_place("ctl_p_1", cap(1), 0);
    let ctl_c0 = net.add_place("ctl_c_0", cap(1), 1);
    let ctl_c1 = net.add_place("ctl_c_1", cap(1), 0);
    let buf = net.add_place("buf", cap(1), 0);

    let push = net.add_transition("push_seq0", None);
    net.add_arc(ArcKind::PtoT, ctl_p0, push, 1);
    net.add_arc(ArcKind::TtoP, ctl_p1, push, 1);
    net.add_arc(ArcKind::TtoP, buf, push, 1);

    let wait = net.add_transition("wait_seq0", None);
    net.add_arc(ArcKind::PtoT, ctl_c0, wait, 1);
    net.add_arc(ArcKind::TtoP, ctl_c1, wait, 1);
    net.add_arc(ArcKind::PtoT, buf, wait, 1);

    check_deadlock_free(&net, &[push, wait])
        .expect("matched producer/consumer must be deadlock-free");
}

/// Same shape but the firing order tries to consume before producing.
/// Stall must be reported at position 0.
#[test]
fn consume_before_produce_stalls_at_position_zero() {
    let mut net = Net::new();
    let ctl_p0 = net.add_place("ctl_p_0", cap(1), 1);
    let ctl_p1 = net.add_place("ctl_p_1", cap(1), 0);
    let ctl_c0 = net.add_place("ctl_c_0", cap(1), 1);
    let ctl_c1 = net.add_place("ctl_c_1", cap(1), 0);
    let buf = net.add_place("buf", cap(1), 0);

    let push = net.add_transition("push_seq0", None);
    net.add_arc(ArcKind::PtoT, ctl_p0, push, 1);
    net.add_arc(ArcKind::TtoP, ctl_p1, push, 1);
    net.add_arc(ArcKind::TtoP, buf, push, 1);

    let wait = net.add_transition("wait_seq0", None);
    net.add_arc(ArcKind::PtoT, ctl_c0, wait, 1);
    net.add_arc(ArcKind::TtoP, ctl_c1, wait, 1);
    net.add_arc(ArcKind::PtoT, buf, wait, 1);

    let err = check_deadlock_free(&net, &[wait, push]).expect_err("must detect stall");
    match err {
        DeadlockError::Stalled {
            transition_name,
            place_name,
            position,
            ..
        } => {
            assert_eq!(transition_name, "wait_seq0");
            assert_eq!(place_name, "buf");
            assert_eq!(position, 0);
        }
        other => panic!("expected Stalled, got {:?}", other),
    }
}

// --------------------------------------------------------------------
// End-to-end: example 02 under split schedule (deadlock-free)
// --------------------------------------------------------------------

/// Example 02 under the `split` schedule must be deadlock-free.
///
/// History: this was the acceptance signal for TASK-0136 / TASK-0139.
/// Before the fix, `transfer_inject` left cross-scope Waits without a
/// matching Push (the `add` consumer sits inside `for i` while
/// `load_input` produces on host at the top level), so the net
/// contained `wait_seq*` transitions whose buffer place was never
/// produced into and the derived firing order stalled at the first
/// such Wait.
///
/// The whole-symbol hoist (Pass A) + global Push finaliser (Pass B)
/// in `transfer_inject` now pair every Wait with a Push at the right
/// scope: `a`/`b` (loop-invariant inputs) get one Wait before the
/// loop and one Push after their producer; `c` (loop output read by
/// `save_output` after the loop) gets one Push after the loop and one
/// Wait before the consumer. The net replays to completion.
///
/// This test is the deadlock pass paying its keep: it pinpoints (and
/// now guards the fix for) a real schedule-lowering bug rather than a
/// synthetic counter-example.
#[test]
fn e2e_example_02_split_is_deadlock_free() {
    let net = pipeline_to_net(
        "02-split-add/prog.algo.nuc",
        "02-split-add/schedules/split.sched.nuc",
    );
    let order = derive_firing_order(&net);
    check_deadlock_free(&net, &order).expect(
        "example 02 split must be deadlock-free after TASK-0136/0139 \
         (whole-symbol Push/Wait pairing); a stall here is a \
         regression in transfer_inject's cross-scope finalisation",
    );
}

// --------------------------------------------------------------------
// End-to-end: example 01 naive (must be deadlock-free)
// --------------------------------------------------------------------

#[test]
fn e2e_example_01_naive_is_deadlock_free() {
    // Single worker, no transfers, no syncs. The derived firing
    // order is just the per-worker control chain; it must run to
    // completion.
    let net = pipeline_to_net(
        "01-elementwise-add/prog.algo.nuc",
        "01-elementwise-add/schedules/naive.sched.nuc",
    );
    let order = derive_firing_order(&net);
    check_deadlock_free(&net, &order).expect("example 01 naive must be deadlock-free");
}

#[test]
fn e2e_example_02_naive_is_deadlock_free() {
    // Single-worker variant of example 02. No cross-worker xfers,
    // so no Push/Wait pairs — the transfer_inject bug doesn't bite.
    let net = pipeline_to_net(
        "02-split-add/prog.algo.nuc",
        "02-split-add/schedules/naive.sched.nuc",
    );
    let order = derive_firing_order(&net);
    check_deadlock_free(&net, &order).expect("example 02 naive must be deadlock-free");
}

// --------------------------------------------------------------------
// Determinism
// --------------------------------------------------------------------

#[test]
fn check_deadlock_free_is_deterministic_on_success() {
    let net = pipeline_to_net(
        "01-elementwise-add/prog.algo.nuc",
        "01-elementwise-add/schedules/naive.sched.nuc",
    );
    let order = derive_firing_order(&net);
    let a = check_deadlock_free(&net, &order);
    let b = check_deadlock_free(&net, &order);
    assert_eq!(a, b);
    a.expect("01 naive deadlock-free");
}

#[test]
fn check_deadlock_free_is_deterministic_on_stall() {
    // Same stall replayed twice yields the same error payload —
    // including the marking_before snapshot (BTreeMap-backed,
    // deterministic iteration).
    //
    // Fixture note: example 02-split used to be the stall fixture
    // here (it deadlocked on the missing-Push bug, TASK-0136/0139).
    // That bug is fixed — example 02-split is now deadlock-free (see
    // `e2e_example_02_split_is_deadlock_free`). To keep testing the
    // *deterministic stall payload* property we use the synthetic
    // unmatched-Wait shape: one `wait_seq0` consuming from a `buf`
    // that no Push ever fills.
    let mut net = Net::new();
    let buf = net.add_place("buf_seq0", cap(1), 0);
    let ctl = net.add_place("ctl_w_0", cap(1), 1);
    let ctl_next = net.add_place("ctl_w_1", cap(1), 0);
    let wait = net.add_transition("wait_seq0", None);
    net.add_arc(ArcKind::PtoT, ctl, wait, 1);
    net.add_arc(ArcKind::PtoT, buf, wait, 1);
    net.add_arc(ArcKind::TtoP, ctl_next, wait, 1);

    let order = [wait];
    let a = check_deadlock_free(&net, &order);
    let b = check_deadlock_free(&net, &order);
    assert_eq!(a, b, "stall payload must be deterministic across runs");
    assert!(matches!(a, Err(DeadlockError::Stalled { .. })));
}

// --------------------------------------------------------------------
// Edge cases
// --------------------------------------------------------------------

#[test]
fn empty_firing_order_passes() {
    // No firings = nothing can stall. Vacuously deadlock-free.
    let mut net = Net::new();
    net.add_place("p", cap(1), 0);
    check_deadlock_free(&net, &[]).expect("no firings is trivially deadlock-free");
}

#[test]
fn unknown_transition_surfaced_as_distinct_error() {
    let net = Net::new();
    let bogus = TransitionId(999);
    let err = check_deadlock_free(&net, &[bogus]).expect_err("must reject unknown transition");
    assert!(matches!(err, DeadlockError::UnknownTransition(t) if t == bogus));
}

#[test]
fn capacity_overflow_surfaced_as_distinct_error() {
    // A firing that overflows a capacity — not a deadlock, but the
    // analysis answer is undefined, so the variant must be
    // CapacityExceeded, not Stalled.
    let mut net = Net::new();
    let src = net.add_place("source", cap(1), 1);
    let buf = net.add_place("buf", cap(1), 0);
    let t = net.add_transition("overflower", None);
    net.add_arc(ArcKind::PtoT, src, t, 1);
    net.add_arc(ArcKind::TtoP, buf, t, 2);

    let err = check_deadlock_free(&net, &[t]).expect_err("overflow surfaces distinctly");
    match err {
        DeadlockError::CapacityExceeded {
            transition_name,
            place_name,
            would_be,
            capacity,
            ..
        } => {
            assert_eq!(transition_name, "overflower");
            assert_eq!(place_name, "buf");
            assert_eq!(would_be, 2);
            assert_eq!(capacity.get(), 1);
        }
        other => panic!("expected CapacityExceeded, got {:?}", other),
    }
}

#[test]
fn stall_at_non_zero_position_reports_correct_index() {
    // Fire two transitions successfully, stall on the third.
    let mut net = Net::new();
    let ctl0 = net.add_place("ctl_0", cap(1), 1);
    let ctl1 = net.add_place("ctl_1", cap(1), 0);
    let ctl2 = net.add_place("ctl_2", cap(1), 0);
    let blocked = net.add_place("blocked", cap(1), 0);

    let step0 = net.add_transition("step0", None);
    net.add_arc(ArcKind::PtoT, ctl0, step0, 1);
    net.add_arc(ArcKind::TtoP, ctl1, step0, 1);

    let step1 = net.add_transition("step1", None);
    net.add_arc(ArcKind::PtoT, ctl1, step1, 1);
    net.add_arc(ArcKind::TtoP, ctl2, step1, 1);

    let step2 = net.add_transition("step2", None);
    net.add_arc(ArcKind::PtoT, ctl2, step2, 1);
    net.add_arc(ArcKind::PtoT, blocked, step2, 1);

    let err = check_deadlock_free(&net, &[step0, step1, step2])
        .expect_err("step2 stalls on empty 'blocked' place");
    match err {
        DeadlockError::Stalled {
            transition_name,
            place_name,
            position,
            ..
        } => {
            assert_eq!(transition_name, "step2");
            assert_eq!(place_name, "blocked");
            assert_eq!(position, 2);
        }
        other => panic!("expected Stalled at position 2, got {:?}", other),
    }
}
