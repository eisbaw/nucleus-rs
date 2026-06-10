//! Behavioural tests for the Petri-net IR (TASK-0025, PRD §8).
//!
//! These exercise the [`nucleus_compiler::petri::Net`] *as a runtime* — we
//! build tiny nets by hand and walk firing sequences. The schedule
//! pass (TASK-0026 onwards) only ever drives the net along a single
//! statically chosen linearisation, but the simulator surface is
//! convenient for tests and for analysis passes.

use std::num::NonZeroU32;

use nucleus_compiler::event::WorkerId;
use nucleus_compiler::petri::{ArcKind, FireError, Net};

fn cap(n: u32) -> Option<NonZeroU32> {
    Some(NonZeroU32::new(n).expect("test capacity must be > 0"))
}

#[test]
fn fire_single_transition_moves_token() {
    // p0 (cap=1, init=1) -> t0 -> p1 (cap=1, init=0)
    let mut net = Net::new();
    let p0 = net.add_place("p0", cap(1), 1);
    let p1 = net.add_place("p1", cap(1), 0);
    let t0 = net.add_transition("t0", None);
    net.add_arc(ArcKind::PtoT, p0, t0, 1);
    net.add_arc(ArcKind::TtoP, p1, t0, 1);

    assert_eq!(net.current_marking.get(p0), 1);
    assert_eq!(net.current_marking.get(p1), 0);

    let after = net.fire(t0).expect("firing should succeed");
    assert_eq!(after.get(p0), 0);
    assert_eq!(after.get(p1), 1);
    assert_eq!(net.current_marking.get(p0), 0);
    assert_eq!(net.current_marking.get(p1), 1);
}

#[test]
fn producer_consumer_buffer_fifo_count() {
    // Classic producer/consumer over a shared buffer place.
    //
    //   producer --[1]--> buffer (cap=2) --[1]--> consumer
    //
    // Fire producer twice -> buffer holds 2 tokens.
    // Fire consumer twice -> buffer is back to empty.
    let mut net = Net::new();
    let buffer = net.add_place("buffer", cap(2), 0);
    let producer = net.add_transition("producer", Some(WorkerId(0)));
    let consumer = net.add_transition("consumer", Some(WorkerId(1)));
    net.add_arc(ArcKind::TtoP, buffer, producer, 1);
    net.add_arc(ArcKind::PtoT, buffer, consumer, 1);

    // Producer fires twice.
    net.fire(producer).expect("first produce");
    assert_eq!(net.current_marking.get(buffer), 1);
    net.fire(producer).expect("second produce");
    assert_eq!(net.current_marking.get(buffer), 2);

    // Consumer fires twice.
    net.fire(consumer).expect("first consume");
    assert_eq!(net.current_marking.get(buffer), 1);
    net.fire(consumer).expect("second consume");
    assert_eq!(net.current_marking.get(buffer), 0);

    // A third consume must fail — no tokens to consume.
    match net.fire(consumer) {
        Err(FireError::NotEnabled {
            place, have, need, ..
        }) => {
            assert_eq!(place, buffer);
            assert_eq!(have, 0);
            assert_eq!(need, 1);
        }
        other => panic!("expected NotEnabled, got {:?}", other),
    }
}

#[test]
fn capacity_exceeded_is_reported() {
    // buffer has capacity 1. Producer alone (no consumer) overflows
    // on the second firing.
    let mut net = Net::new();
    let buffer = net.add_place("buffer", cap(1), 0);
    let producer = net.add_transition("producer", None);
    net.add_arc(ArcKind::TtoP, buffer, producer, 1);

    net.fire(producer).expect("first produce fits");
    match net.fire(producer) {
        Err(FireError::CapacityExceeded {
            place,
            would_be,
            capacity,
            ..
        }) => {
            assert_eq!(place, buffer);
            assert_eq!(would_be, 2);
            assert_eq!(capacity.get(), 1);
        }
        other => panic!("expected CapacityExceeded, got {:?}", other),
    }

    // And the failed firing must not have mutated the marking.
    assert_eq!(net.current_marking.get(buffer), 1);
}

#[test]
fn not_enabled_when_input_short() {
    // Transition needs 3 tokens from p; p starts with 1.
    let mut net = Net::new();
    let p = net.add_place("p", cap(5), 1);
    let t = net.add_transition("t", None);
    net.add_arc(ArcKind::PtoT, p, t, 3);

    match net.fire(t) {
        Err(FireError::NotEnabled {
            have, need, place, ..
        }) => {
            assert_eq!(have, 1);
            assert_eq!(need, 3);
            assert_eq!(place, p);
        }
        other => panic!("expected NotEnabled, got {:?}", other),
    }
}

#[test]
fn enabled_transitions_filters_by_capacity_and_availability() {
    // Two transitions sharing a place; only one can fire at a time.
    //   p_full (cap=1, init=1) --[1]--> drain
    //   fill --[1]--> p_full
    //
    // From the initial marking:
    //   drain is enabled (has the token).
    //   fill is NOT enabled (would overflow p_full).
    let mut net = Net::new();
    let p_full = net.add_place("p_full", cap(1), 1);
    let drain = net.add_transition("drain", None);
    let fill = net.add_transition("fill", None);
    net.add_arc(ArcKind::PtoT, p_full, drain, 1);
    net.add_arc(ArcKind::TtoP, p_full, fill, 1);

    let enabled = net.enabled_transitions(&net.current_marking);
    assert!(enabled.contains(&drain), "drain should be enabled");
    assert!(
        !enabled.contains(&fill),
        "fill should be disabled by capacity"
    );

    net.fire(drain).expect("drain fires");
    let enabled = net.enabled_transitions(&net.current_marking);
    assert!(
        enabled.contains(&fill),
        "fill enabled after drain frees space"
    );
    assert!(
        !enabled.contains(&drain),
        "drain disabled after consuming token"
    );
}

#[test]
fn reset_to_initial_restores_marking() {
    let mut net = Net::new();
    let p = net.add_place("p", cap(3), 2);
    let t = net.add_transition("t", None);
    net.add_arc(ArcKind::PtoT, p, t, 1);

    net.fire(t).expect("fire");
    net.fire(t).expect("fire");
    assert_eq!(net.current_marking.get(p), 0);

    net.reset_to_initial();
    assert_eq!(net.current_marking.get(p), 2);
}

#[test]
fn dot_output_mentions_node_names() {
    let mut net = Net::new();
    let buf = net.add_place("buffer", cap(2), 1);
    let prod = net.add_transition("producer", Some(WorkerId(7)));
    let cons = net.add_transition("consumer", None);
    net.add_arc(ArcKind::TtoP, buf, prod, 1);
    net.add_arc(ArcKind::PtoT, buf, cons, 2);

    let dot = net.serialize_to_dot();

    // Spot-check: names, the digraph keyword, and the weighted-arc
    // label (weight=2). The exact pixel layout is Graphviz's problem.
    assert!(
        dot.starts_with("digraph petri"),
        "should be a digraph: got {}",
        dot
    );
    assert!(dot.contains("buffer"), "missing place name in {}", dot);
    assert!(dot.contains("producer"), "missing producer name in {}", dot);
    assert!(dot.contains("consumer"), "missing consumer name in {}", dot);
    assert!(
        dot.contains("w7"),
        "missing worker tag for producer in {}",
        dot
    );
    assert!(
        dot.contains("label=\"2\""),
        "weight=2 arc should be labelled in {}",
        dot
    );
}

#[test]
fn unknown_transition_id_is_an_error_not_a_panic() {
    let net = Net::new();
    // Hand-crafted id that the empty net never issued.
    let bogus = nucleus_compiler::petri::TransitionId(999);
    // We need a fresh mut handle:
    let mut net = net;
    match net.fire(bogus) {
        Err(FireError::UnknownTransition(t)) => assert_eq!(t.0, 999),
        other => panic!("expected UnknownTransition, got {:?}", other),
    }
}

#[test]
fn weighted_arcs_consume_multiple_tokens() {
    // p (cap=10, init=5) --[3]--> t --[2]--> q (cap=10, init=0)
    // After one fire: p has 2, q has 2.
    let mut net = Net::new();
    let p = net.add_place("p", cap(10), 5);
    let q = net.add_place("q", cap(10), 0);
    let t = net.add_transition("t", None);
    net.add_arc(ArcKind::PtoT, p, t, 3);
    net.add_arc(ArcKind::TtoP, q, t, 2);

    let m = net.fire(t).expect("fire");
    assert_eq!(m.get(p), 2);
    assert_eq!(m.get(q), 2);

    // Second fire would need 3 from p but only 2 remain.
    assert!(matches!(net.fire(t), Err(FireError::NotEnabled { .. })));
}

/// TASK-0455.10: the gate passes were changed to replay on a BORROWED
/// `&Net` plus an owned [`Marking`] via [`Net::fire_marking`] instead of
/// cloning the whole net and calling [`Net::fire_in_place`] on its
/// `current_marking`. The two paths MUST be byte-for-byte equivalent —
/// same committed marking on success, same `FireError` (untouched
/// marking) on failure — or the clone elimination silently changed gate
/// verdicts. This pins that equivalence over a representative firing
/// sequence covering success, a capacity-overflow failure, and an
/// enabling-stall failure.
#[test]
fn fire_marking_matches_fire_in_place_step_for_step() {
    use nucleus_compiler::petri::Marking;

    // Build a small net with a producer, a cap=1 buffer, and a consumer.
    //   produce --[1]--> buf(cap=1) --[1]--> consume
    // Sequence: produce (ok), produce again (CapacityExceeded), consume
    // (ok), consume again (NotEnabled).
    let mut net = Net::new();
    let ctl_p = net.add_place("ctl_p", cap(4), 4);
    let ctl_c = net.add_place("ctl_c", cap(4), 4);
    let buf = net.add_place("buf", cap(1), 0);
    let produce = net.add_transition("produce", Some(WorkerId(0)));
    let consume = net.add_transition("consume", Some(WorkerId(1)));
    net.add_arc(ArcKind::PtoT, ctl_p, produce, 1);
    net.add_arc(ArcKind::TtoP, buf, produce, 1);
    net.add_arc(ArcKind::PtoT, ctl_c, consume, 1);
    net.add_arc(ArcKind::PtoT, buf, consume, 1);

    let seq = [produce, produce, consume, consume];

    // Path A: the historical clone-and-fire-in-place path.
    let mut clone = net.clone();
    clone.reset_to_initial();
    let index_a = net.build_arc_index();
    let mut results_a = Vec::new();
    let mut markings_a = Vec::new();
    for &t in &seq {
        let r = clone.fire_in_place(t, &index_a);
        results_a.push(r);
        markings_a.push(clone.current_marking.clone());
    }

    // Path B: the borrowed-net + owned-marking path the gate now uses.
    let index_b = net.build_arc_index();
    let mut marking_b: Marking = net.initial_marking.clone();
    let mut results_b = Vec::new();
    let mut markings_b = Vec::new();
    for &t in &seq {
        let r = net.fire_marking(t, &mut marking_b, &index_b);
        results_b.push(r);
        markings_b.push(marking_b.clone());
    }

    assert_eq!(
        results_a, results_b,
        "fire_marking must produce the same Result sequence as fire_in_place"
    );
    assert_eq!(
        markings_a, markings_b,
        "fire_marking must leave the same marking after each step (incl. \
         untouched marking on the Err steps) as fire_in_place"
    );

    // Spot-check the expected verdicts so a regression in BOTH paths
    // (which would keep them equal but wrong) is still caught.
    assert!(results_b[0].is_ok(), "first produce fills the cap=1 buffer");
    assert!(
        matches!(results_b[1], Err(FireError::CapacityExceeded { .. })),
        "second produce overflows the cap=1 buffer"
    );
    assert!(results_b[2].is_ok(), "consume drains the buffer");
    assert!(
        matches!(results_b[3], Err(FireError::NotEnabled { .. })),
        "second consume stalls on the now-empty buffer"
    );
}
