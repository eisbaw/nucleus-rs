//! Integration tests for the boundedness analysis pass
//! (TASK-0028, PRD §8.2).
//!
//! Strategy
//! --------
//!
//! - **Synthetic negative case**: a 2-token push into a capacity-1
//!   place. `check_bounded` must reject and the error must name the
//!   offending place + transition.
//!
//! - **Synthetic positive case**: a small producer/consumer net with
//!   matched capacity. `check_bounded` must accept.
//!
//! - **End-to-end**: build the global net for example 02 under the
//!   `split` schedule. Derive a firing order and check boundedness.
//!   v2's restricted nets are bounded by construction, so a real
//!   example must pass.
//!
//! - **Determinism**: the same input produces the same result on
//!   repeated calls.

use std::num::NonZeroU32;

use nucleus_compiler::algo::{lower_algo, parse_algo};
use nucleus_compiler::link;
use nucleus_compiler::passes::acfg_to_petri::acfg_to_net;
use nucleus_compiler::passes::boundedness::{check_bounded, derive_firing_order, BoundednessError};
use nucleus_compiler::passes::sync_inject::inject_syncs;
use nucleus_compiler::passes::transfer_inject::inject_transfers;
use nucleus_compiler::petri::{ArcKind, Net};
use nucleus_compiler::sched::{lower_sched, parse_sched};

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
    let acfg = nucleus_compiler::acfg::build_acfg(&linked).expect("build_acfg");
    let acfg = inject_syncs(acfg).expect("inject_syncs");
    let acfg = inject_transfers(&linked, acfg).expect("inject_transfers");
    acfg_to_net(&acfg)
}

// --------------------------------------------------------------------
// Synthetic negative case: 2-token push into a capacity-1 place
// --------------------------------------------------------------------

/// Build a tiny net with:
///   - a "source" place pre-marked with 1 token (cap 1),
///   - a "buf" place with capacity = 1,
///   - one transition that consumes from source and deposits *2*
///     tokens into buf.
///
/// Firing the transition once would push buf to 2 tokens, which
/// exceeds its capacity. `check_bounded` must reject and name buf
/// and the transition.
#[test]
fn two_token_push_into_cap1_place_is_rejected() {
    let mut net = Net::new();
    let src = net.add_place("source", cap(1), 1);
    let buf = net.add_place("buf", cap(1), 0);
    let t = net.add_transition("overflower", None);
    net.add_arc(ArcKind::PtoT, src, t, 1);
    // Weight-2 arc — the offending production.
    net.add_arc(ArcKind::TtoP, buf, t, 2);

    let err = check_bounded(&net, &[t]).expect_err("must reject");
    match err {
        BoundednessError::CapacityExceeded {
            place,
            place_name,
            transition,
            transition_name,
            would_be,
            capacity,
            ..
        } => {
            assert_eq!(place, buf);
            assert_eq!(place_name, "buf");
            assert_eq!(transition, t);
            assert_eq!(transition_name, "overflower");
            assert_eq!(would_be, 2);
            assert_eq!(capacity.get(), 1);
        }
        other => panic!("expected CapacityExceeded, got {:?}", other),
    }
}

// --------------------------------------------------------------------
// Synthetic positive case: matched producer/consumer
// --------------------------------------------------------------------

/// One producer, one consumer, capacity-1 buffer, alternating
/// firings. The order must be [produce, consume, produce, consume]
/// — anything else would queue 2 tokens in `buf`.
///
/// We test that `check_bounded` accepts this matched flow.
#[test]
fn matched_producer_consumer_passes() {
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

    // Strictly alternating order respects buf's capacity.
    let order = vec![produce0, consume0, produce1, consume1];
    check_bounded(&net, &order).expect("matched producer/consumer should be bounded");
}

/// Same shape as above but the firing order produces twice before
/// consuming. That would put 2 tokens in a capacity-1 buffer.
/// `check_bounded` must reject and the error must point at `buf` and
/// `produce1`.
#[test]
fn back_to_back_produce_into_cap1_buffer_is_rejected() {
    let mut net = Net::new();
    let ctl_p0 = net.add_place("ctl_p_0", cap(1), 1);
    let ctl_p1 = net.add_place("ctl_p_1", cap(1), 0);
    let ctl_p2 = net.add_place("ctl_p_2", cap(1), 0);
    let buf = net.add_place("buf", cap(1), 0);

    let produce0 = net.add_transition("produce0", None);
    net.add_arc(ArcKind::PtoT, ctl_p0, produce0, 1);
    net.add_arc(ArcKind::TtoP, ctl_p1, produce0, 1);
    net.add_arc(ArcKind::TtoP, buf, produce0, 1);

    let produce1 = net.add_transition("produce1", None);
    net.add_arc(ArcKind::PtoT, ctl_p1, produce1, 1);
    net.add_arc(ArcKind::TtoP, ctl_p2, produce1, 1);
    net.add_arc(ArcKind::TtoP, buf, produce1, 1);

    let err =
        check_bounded(&net, &[produce0, produce1]).expect_err("back-to-back produce overflows");
    match err {
        BoundednessError::CapacityExceeded {
            place_name,
            transition_name,
            would_be,
            capacity,
            ..
        } => {
            assert_eq!(place_name, "buf");
            assert_eq!(transition_name, "produce1");
            assert_eq!(would_be, 2);
            assert_eq!(capacity.get(), 1);
        }
        other => panic!("expected CapacityExceeded, got {:?}", other),
    }
}

// --------------------------------------------------------------------
// End-to-end: example 02 under split schedule
// --------------------------------------------------------------------

#[test]
fn e2e_example_02_split_never_overflows_capacity() {
    // Example 02 under the `split` schedule is bounded by construction
    // (PRD §8.4). We assert the *boundedness* property directly: the
    // derived firing order replays to completion with no place ever
    // exceeding its capacity — i.e. `check_bounded` returns `Ok(())`.
    //
    // Caveat RESOLVED (TASK-0368): an earlier version of this test
    // (TASK-0028 era) tolerated `BoundednessError::InvalidFiringOrder`
    // because `transfer_inject` did not splice a Push placeholder when
    // the matching Wait was inside a `Repeat` body whose producer sat
    // in the outer sequence — the net carried `wait_seq*` transitions
    // with no `push_seq*` peers and the first Wait fired against an
    // empty buffer place. TASK-0136/0139's whole-symbol Push/Wait
    // pairing fixed that; today's rebuilt net is well-formed and
    // replays fully. The companion deadlock test
    // (`tests/deadlock.rs::e2e_example_02_split_is_deadlock_free`)
    // already asserts the no-stall half; this test now asserts the
    // no-overflow half with the same strictness.
    //
    // This tightening is load-bearing: the production driver runs
    // `check_net_sound` (boundedness + deadlock) on every build as of
    // TASK-0368, so accepting a stalled firing order here would mask a
    // regression the shipping gate must reject.
    let net = pipeline_to_net(
        "02-split-add/prog.algo.nuc",
        "02-split-add/schedules/split.sched.nuc",
    );
    let order = derive_firing_order(&net);
    check_bounded(&net, &order).expect(
        "example 02 split must be bounded by construction and replay to \
         completion (TASK-0136/0139 whole-symbol Push/Wait pairing); an \
         InvalidFiringOrder or CapacityExceeded here is a regression in \
         transfer_inject's cross-scope finalisation",
    );
}

#[test]
fn e2e_example_02_naive_is_bounded() {
    // Single-worker schedule. No buffer places at all, so boundedness
    // is trivially satisfied — but the check should still succeed.
    let net = pipeline_to_net(
        "02-split-add/prog.algo.nuc",
        "02-split-add/schedules/naive.sched.nuc",
    );
    let order = derive_firing_order(&net);
    check_bounded(&net, &order).expect("example 02 naive must be bounded by construction");
}

#[test]
fn e2e_example_01_naive_is_bounded() {
    // Smoke: a fully end-to-end runnable example must clear the
    // boundedness check. No buffer places, no cross-worker xfers.
    let net = pipeline_to_net(
        "01-elementwise-add/prog.algo.nuc",
        "01-elementwise-add/schedules/naive.sched.nuc",
    );
    let order = derive_firing_order(&net);
    check_bounded(&net, &order).expect("example 01 naive must be bounded by construction");
}

#[test]
fn derive_firing_order_preserves_source_order_on_nonpipelined_fixture() {
    // TASK-0213 regression: when no place carries an initial marking
    // that forces a reorder, derive_firing_order must return the
    // plain source order (TransitionId(0), TransitionId(1), ...).
    //
    // This pins the behaviour for every pre-TASK-0213 fixture: the
    // marking-aware algorithm degenerates to source-order whenever
    // source-order is already a legal firing sequence.
    //
    // Example 02 naive is the simplest example with multiple
    // transitions in source order; it has no buffer places and no
    // initial markings beyond the per-worker control places (each
    // pre-marked with exactly 1 to enable that worker's first step).
    use nucleus_compiler::petri::TransitionId;
    let net = pipeline_to_net(
        "02-split-add/prog.algo.nuc",
        "02-split-add/schedules/naive.sched.nuc",
    );
    let order = derive_firing_order(&net);
    let expected: Vec<TransitionId> = net.transitions.iter().map(|t| t.id).collect();
    assert_eq!(
        order, expected,
        "non-pipelined fixture must keep plain insertion order"
    );
}

// --------------------------------------------------------------------
// Determinism
// --------------------------------------------------------------------

#[test]
fn check_bounded_is_deterministic() {
    // Same input twice yields identical outputs. We use the naive
    // schedule (which actually fires to completion) so the assertion
    // covers both the order-derivation and the bounded-replay paths.
    let net = pipeline_to_net(
        "02-split-add/prog.algo.nuc",
        "02-split-add/schedules/naive.sched.nuc",
    );
    let order_a = derive_firing_order(&net);
    let order_b = derive_firing_order(&net);
    assert_eq!(
        order_a, order_b,
        "derive_firing_order must be deterministic"
    );

    let r1 = check_bounded(&net, &order_a);
    let r2 = check_bounded(&net, &order_a);
    assert_eq!(r1, r2, "check_bounded must be deterministic");
    r1.expect("naive 02 must be bounded");
}

#[test]
fn check_bounded_on_overflow_is_deterministic() {
    // Same overflow case run twice yields the same error payload.
    let mut net = Net::new();
    let src = net.add_place("source", cap(1), 1);
    let buf = net.add_place("buf", cap(1), 0);
    let t = net.add_transition("overflower", None);
    net.add_arc(ArcKind::PtoT, src, t, 1);
    net.add_arc(ArcKind::TtoP, buf, t, 2);

    let e1 = check_bounded(&net, &[t]).expect_err("rejects");
    let e2 = check_bounded(&net, &[t]).expect_err("rejects");
    assert_eq!(e1, e2);
}

// --------------------------------------------------------------------
// Edge cases
// --------------------------------------------------------------------

#[test]
fn empty_firing_order_passes() {
    // No firings = nothing can overflow. Vacuously bounded.
    let mut net = Net::new();
    net.add_place("p", cap(1), 0);
    check_bounded(&net, &[]).expect("no firings is trivially bounded");
}

#[test]
fn unknown_transition_surfaced_as_distinct_error() {
    // Pass an out-of-range transition id. Boundedness is undefined,
    // and the caller must see a distinct error rather than Ok.
    use nucleus_compiler::petri::TransitionId;
    let net = Net::new();
    let bogus = TransitionId(999);
    let err = check_bounded(&net, &[bogus]).expect_err("must reject unknown transition");
    assert!(matches!(err, BoundednessError::UnknownTransition(t) if t == bogus));
}

#[test]
fn illegal_firing_order_surfaced_as_invalid() {
    // A transition that needs a token its input place doesn't hold.
    // The error variant must be InvalidFiringOrder, not CapacityExceeded.
    let mut net = Net::new();
    let src = net.add_place("source", cap(1), 0); // empty
    let dst = net.add_place("dst", cap(1), 0);
    let t = net.add_transition("needs_token", None);
    net.add_arc(ArcKind::PtoT, src, t, 1);
    net.add_arc(ArcKind::TtoP, dst, t, 1);

    let err = check_bounded(&net, &[t]).expect_err("source has no token");
    match err {
        BoundednessError::InvalidFiringOrder {
            transition_name,
            place_name,
            have,
            need,
            ..
        } => {
            assert_eq!(transition_name, "needs_token");
            assert_eq!(place_name, "source");
            assert_eq!(have, 0);
            assert_eq!(need, 1);
        }
        other => panic!("expected InvalidFiringOrder, got {:?}", other),
    }
}

// --------------------------------------------------------------------
// TASK-0219: path-1 marking-aware firing-order coverage
//
// `derive_firing_order` has two layers: source-order with marking-aware
// reordering (path 1), and a stuck-state fallback that appends
// remaining transitions in source order so `check_bounded` surfaces
// the violation cleanly. Until TASK-0219, neither layer had a direct
// test — every in-tree fixture either has source-order legal already
// (TASK-0213's path-2 elision applies) or doesn't construct nets with
// pre-marked-at-capacity buffer places. These tests cover the two
// path-1 behaviours on hand-built synthetic nets so the defensive
// logic is no longer "dead code with no test".
// --------------------------------------------------------------------

/// Path-1 marking-aware REORDER: source-order picks a transition first
/// that would overflow under the *initial marking*, but a later
/// transition is firable from the same marking and drains the place;
/// after that, the originally-first transition becomes firable too. A
/// legal firing sequence exists; `derive_firing_order` must discover
/// it by picking firable transitions in source order at each step
/// (NOT by replaying source order blindly).
///
/// Setup:
///   - "ctrl" : cap=2, initial=2  — enables both transitions.
///   - "buf"  : cap=1, initial=1  — FULL at start.
///   - T1 "produce" : consume 1 from ctrl, produce 1 into buf  (source idx 0).
///   - T2 "consume" : consume 1 from buf                       (source idx 1).
///
/// Plain source-order [T1, T2] would overflow buf on the first step.
/// Marking-aware path-1 picks [T2, T1]: T2 drains buf to 0; then T1
/// can produce into buf without overflow. `check_bounded` on the
/// path-1 output must succeed.
#[test]
fn derive_firing_order_reorders_under_initial_marking_pressure() {
    let mut net = Net::new();
    let ctrl = net.add_place("ctrl", cap(2), 2);
    let buf = net.add_place("buf", cap(1), 1);

    // T1 first in source order — would overflow if fired now.
    let produce = net.add_transition("produce", None);
    net.add_arc(ArcKind::PtoT, ctrl, produce, 1);
    net.add_arc(ArcKind::TtoP, buf, produce, 1);

    // T2 second — drains buf, making room for T1.
    let consume = net.add_transition("consume", None);
    net.add_arc(ArcKind::PtoT, buf, consume, 1);
    net.add_arc(ArcKind::PtoT, ctrl, consume, 1);

    let order = derive_firing_order(&net);
    // Marking-aware: consume FIRST (drains buf), then produce (refills).
    assert_eq!(
        order,
        vec![consume, produce],
        "path-1 must pick the firable consumer first when the producer \
         would overflow under the initial marking; got {:?}",
        order
    );
    // The path-1 output IS a legal firing sequence.
    check_bounded(&net, &order).expect("path-1 order must be legal");
}

/// Path-1 STUCK-STATE FALLBACK: a net where source-order isn't legal
/// AND no legal interleaving exists from the initial marking.
/// `derive_firing_order` fires what it CAN, then appends the
/// unfirable leftover transitions in source order — so `check_bounded`
/// can surface a precise violation at the first stuck transition,
/// not silently truncate the firing trace.
///
/// Setup:
///   - "ctrl" : cap=1, initial=1  — enables T1.
///   - "buf"  : cap=1, initial=0  — empty.
///   - T1 "fill_partial" : consume ctrl, produce 1 into buf  (idx 0).
///   - T2 "overfill"     : consume nothing, produce 1 into buf  (idx 1).
///       (Synthetic: T2 has no input arc on any place — always firable
///       from the marking-readiness perspective, BUT after T1 fires
///       buf is at cap=1, so T2 would overflow. This wedges the net.)
///
/// Expected trace:
///   - Step 1: T1 firable (ctrl=1, buf=0→1). Fired. buf=1.
///   - Step 2: T2 unfirable (buf=1 + 1 = 2 > cap=1). No other firable
///     transition. Stuck.
///   - Path-1 appends T2 in source order: order = [T1, T2].
///   - `check_bounded([T1, T2])` re-fires from initial; T1 ok; T2
///     trips `CapacityExceeded { place: buf, transition: T2 }`.
#[test]
fn derive_firing_order_appends_stuck_leftovers_so_check_bounded_diagnoses() {
    let mut net = Net::new();
    let ctrl = net.add_place("ctrl", cap(1), 1);
    let buf = net.add_place("buf", cap(1), 0);

    let fill_partial = net.add_transition("fill_partial", None);
    net.add_arc(ArcKind::PtoT, ctrl, fill_partial, 1);
    net.add_arc(ArcKind::TtoP, buf, fill_partial, 1);

    let overfill = net.add_transition("overfill", None);
    // No PtoT arc — `overfill` has no input dependency, so it is
    // always "enabled" from a token-availability standpoint. The
    // capacity check on its TtoP arc is what wedges it after T1.
    net.add_arc(ArcKind::TtoP, buf, overfill, 1);

    let order = derive_firing_order(&net);
    // Path-1 fires T1, then appends T2 in source order despite T2
    // being unfirable from the post-T1 marking.
    assert_eq!(
        order,
        vec![fill_partial, overfill],
        "stuck-state fallback must append the unfirable leftover in \
         source order so check_bounded can pinpoint it; got {:?}",
        order
    );
    // check_bounded surfaces the precise overflow at the stuck site.
    let err = check_bounded(&net, &order).expect_err("must reject");
    match err {
        BoundednessError::CapacityExceeded {
            transition_name,
            place_name,
            ..
        } => {
            assert_eq!(transition_name, "overfill");
            assert_eq!(place_name, "buf");
        }
        other => panic!("expected CapacityExceeded at overfill, got {:?}", other),
    }
}

// --------------------------------------------------------------------
// TASK-0377 perf-regression pin: the gate must stay near-linear
// --------------------------------------------------------------------

/// Catch a re-introduction of the O(T·A) all-arcs-scan in
/// `Net::fire` (TASK-0377).
///
/// ## Why this test exists
///
/// The cycle-217 per-build Petri gate (`check_net_sound` =
/// `derive_firing_order` + `check_bounded` + `check_deadlock_free`)
/// originally fired each of ~T transitions by scanning ALL arcs twice
/// per fire — O(A) per fire, O(T·A) per pass. On 07-matmul/distributed8
/// (T=4149, A=65722) the gate cost was ~435 ms (99% of build).
/// TASK-0377 added a per-transition arc-adjacency index so each fire is
/// O(deg(t)); the gate dropped to ~16 ms (27×).
///
/// This test builds a large net (T transitions, A = 2·T arcs) on which
/// the OLD O(T·A) gate did ~T·A·3 ≈ 1.5G arc comparisons across the
/// three passes. T is sized so the separation is unambiguous and
/// machine-/profile-independent in BOTH directions. The original
/// T=4000 sizing was too small — the old O(T·A) code ran only
/// ~0.83–1.38 s there (measured), straddling a 1 s ceiling, so it
/// could PASS on a fast box with the regression present (TASK-0377
/// architect P2 finding). Two measured anchors fix the sizing:
///
/// - NEW (near-linear) gate, dev profile, this box: ~45 ms at T=4000,
///   scaling linearly to ~210 ms at T=16000 (45→98→120→144→210 ms for
///   T=4000/8000/10000/12000/16000). Release is faster still.
/// - OLD (O(T·A), scales as T² since A=2·T) gate, measured at T=4000:
///   ~0.83–1.38 s. ×(16000/4000)² = ×16 ⇒ ~13–22 s at T=16000.
///
/// So the 2 s ceiling below clears the near-linear gate by ~10× (even
/// a 5× slower/loaded box stays ~1 s < 2 s) while an accidental return
/// to O(T·A) overshoots by ~7–11× (even a hypothetical 5× faster box
/// keeps the old code at ~2.6 s+). Both margins are wide and two-sided.
///
/// ## Manual macro-benchmark (the headline TASK-0377 number)
///
/// The end-to-end gate cost on the real worst-case net is measured by
/// timing the prebuilt release binary directly (NOT `cargo run`):
///
/// ```text
/// LC_ALL=C
/// BIN=nucleus/target/release/nucleus
/// E=nuc-nucleus/examples/07-matmul
/// # 30 reps, wall clock via $EPOCHREALTIME / 30:
/// "$BIN" build --algo "$E/prog.algo.nuc" \
///   --sched "$E/schedules/distributed8.sched.nuc" \
///   --kernels "$E/kernels.rs" --backend pthreads-sync --out "$(mktemp -d)"
/// ```
///
/// Before TASK-0377: build ≈ 439 ms (gate ≈ 435 ms).
/// After  TASK-0377: build ≈ 27 ms  (gate ≈ 16 ms).
#[test]
fn gate_stays_near_linear_under_large_net() {
    use nucleus_compiler::passes::net_soundness::check_net_sound;

    // T independent fire-once transitions. Each consumes 1 token from
    // its own pre-marked source place and deposits 1 token into its own
    // sink place: deg(t) = 2, source-order is a legal firing order, and
    // the whole net is bounded + deadlock-free. A = 2·T arcs.
    const T: usize = 16000;
    let mut net = Net::new();
    let mut ts = Vec::with_capacity(T);
    for i in 0..T {
        let src = net.add_place(format!("src_{i}"), cap(1), 1);
        let snk = net.add_place(format!("snk_{i}"), cap(1), 0);
        let t = net.add_transition(format!("t_{i}"), None);
        net.add_arc(ArcKind::PtoT, src, t, 1);
        net.add_arc(ArcKind::TtoP, snk, t, 1);
        ts.push(t);
    }

    // Run the real production gate entry point (all three passes).
    let start = std::time::Instant::now();
    check_net_sound(&net).expect("large fan net is sound");
    let elapsed = start.elapsed();

    // OLD O(T·A) here is ~T²·const ≈ 13–22 s (see docstring); the
    // near-linear gate is ~0.2 s in dev / faster in release. 2 s is a
    // two-sided non-flaky ceiling: ~10× above the near-linear gate,
    // ~7–11× below an O(T·A) regression.
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "TASK-0377 perf-regression: check_net_sound on a {T}-transition / {} -arc net \
         took {elapsed:?} (> 2 s) — the per-transition arc index in `Net::fire_in_place` \
         was likely lost, reintroducing the O(T·A) all-arcs scan",
        2 * T
    );
}
