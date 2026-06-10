//! The firing-order invariant, stated and pinned as a TESTED PROPERTY
//! (TASK-0455.01 Stage 1, architecture-review cross-link (1)).
//!
//! ## Why this file exists
//!
//! [`derive_firing_order`](nucleus_compiler::derive_firing_order) is the
//! linearisation that the whole Petri-net soundness gate
//! ([`check_net_sound`](nucleus_compiler::check_net_sound)) replays — it
//! is the *single* order against which boundedness and deadlock are
//! decided. The symbolic communicating-net gate (TASK-0455.01) proves
//! soundness WITHOUT building that order; for the symbolic proof to be
//! equivalent to the expanded replay, it must be proved against a
//! *stated* invariant of `derive_firing_order`, not against the
//! incidental shape of its current implementation (greedy
//! first-firable-in-source-order with a stuck-leftover append). This file
//! states that invariant and pins it, so a future refactor of
//! `derive_firing_order` that still satisfies the invariant keeps the
//! symbolic gate's soundness argument valid, and one that BREAKS it trips
//! here.
//!
//! ## The invariant (what every conforming `derive_firing_order` satisfies)
//!
//! Let `N = net.transitions.len()` and `O = derive_firing_order(net)`.
//!
//! **(I1) Total permutation.** `O` lists every [`TransitionId`] in the
//! net exactly once. (No transition is dropped or duplicated.)
//!
//! **(I2) Maximal legal prefix.** There is a prefix length `L` such that
//! replaying `O[0..L]` from `net.initial_marking` never stalls and never
//! overflows (each step is *firable*: enabled AND capacity-respecting),
//! and `L` is maximal — at the marking reached after `O[0..L]`, NO
//! not-yet-fired transition is firable. For a sound net `L == N` (the
//! order is a complete legal firing sequence); for an unsound net `L < N`
//! and `O[L]` is exactly the transition the boundedness/deadlock passes
//! diagnose the stall/overflow at.
//!
//! **(I3) Greedy first-firable.** For each `i < L`, `O[i]` is the FIRST
//! transition in source (id) order, among those not in `O[0..i]`, that is
//! firable under the marking reached after `O[0..i]`.
//!
//! **(I4) Determinism.** `derive_firing_order(net)` returns the identical
//! `Vec` on repeated calls for the same `net`.
//!
//! **(I5) Source-order degeneration.** If plain source order
//! `[T0, T1, ..., T(N-1)]` is itself a legal firing sequence from the
//! initial marking (the case for every net with no nonzero *buffer*
//! initial marking — see `derive_firing_order`'s docstring), then
//! `O == [T0, ..., T(N-1)]`.
//!
//! ## What the symbolic gate uses
//!
//! The symbolic communicating-net gate (`net_soundness_symbolic_comm`)
//! relies on (I2)+(I3): for a single-shot buffer place (one Push, one
//! Wait, push before wait in source order, no pre-mark) the greedy order
//! fires the Push (depositing one token, within capacity `>= 1`) and then
//! the Wait (draining it) — so the buffer's peak occupancy is exactly 1
//! and never exceeds capacity, the Wait never stalls (its producing Push
//! is earlier in the order by (I3)+(I5)), and there is a single consumer
//! (no conflict). That argument is sound for ANY `derive_firing_order`
//! satisfying the invariant above, which is what this file guarantees.

use std::num::NonZeroU32;

use nucleus_compiler::algo::{lower_algo, parse_algo};
use nucleus_compiler::link;
use nucleus_compiler::passes::acfg_to_petri::acfg_to_net;
use nucleus_compiler::passes::sync_inject::inject_syncs;
use nucleus_compiler::passes::transfer_inject::inject_transfers;
use nucleus_compiler::petri::{ArcKind, Marking, Net, TransitionId};
use nucleus_compiler::sched::{lower_sched, parse_sched};
use nucleus_compiler::{acfg::ACFG, derive_firing_order};

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

fn pipeline_to_acfg(algo_rel: &str, sched_rel: &str) -> ACFG {
    let algo = lower_algo(&parse_algo(&read_example(algo_rel)).expect("algo parse"))
        .expect("algo lower");
    let sched = lower_sched(&parse_sched(&read_example(sched_rel)).expect("sched parse"))
        .expect("sched lower");
    let linked = link::link(algo, sched).expect("link");
    let acfg = nucleus_compiler::acfg::build_acfg(&linked).expect("build_acfg");
    let acfg = inject_syncs(acfg).expect("inject_syncs");
    inject_transfers(&linked, acfg).expect("inject_transfers")
}

fn pipeline_to_net(algo_rel: &str, sched_rel: &str) -> Net {
    acfg_to_net(&pipeline_to_acfg(algo_rel, sched_rel))
}

/// Is transition `t` firable under `marking`? (Enabled AND would not
/// overflow any output place.) Mirrors the firability oracle inside
/// `derive_firing_order` (`fire_marking(...).is_ok()`), restated here at
/// the test level so the property is checked against the *specification*
/// of firability, not by calling the function under test.
fn firable(net: &Net, marking: &Marking, t: TransitionId, index: &nucleus_compiler::petri::ArcIndex) -> bool {
    let mut m = marking.clone();
    net.fire_marking(t, &mut m, index).is_ok()
}

/// The maximal-legal-prefix length `L` of an order, and the marking
/// reached after `O[0..L]`. Replays `O` step by step, stopping at the
/// first non-firable step.
fn legal_prefix_len(net: &Net, order: &[TransitionId]) -> (usize, Marking) {
    let index = net.build_arc_index();
    let mut marking = net.initial_marking.clone();
    for (i, &tid) in order.iter().enumerate() {
        if net.fire_marking(tid, &mut marking, &index).is_err() {
            return (i, marking);
        }
    }
    (order.len(), marking)
}

/// Assert the full firing-order invariant (I1)-(I3) on `net`. (I4)/(I5)
/// are pinned by dedicated tests below since they need extra inputs.
fn assert_firing_order_invariant(net: &Net) {
    let order = derive_firing_order(net);
    let n = net.transitions.len();
    let index = net.build_arc_index();

    // (I1) Total permutation.
    assert_eq!(order.len(), n, "(I1) order length must equal transition count");
    let mut seen = vec![false; n];
    for &t in &order {
        let idx = t.0 as usize;
        assert!(idx < n, "(I1) order references unknown transition {t:?}");
        assert!(!seen[idx], "(I1) transition {t:?} fired twice");
        seen[idx] = true;
    }
    assert!(seen.iter().all(|&b| b), "(I1) every transition must appear");

    // (I2) Maximal legal prefix: replay until first non-firable step;
    // that point L must have NO remaining firable transition.
    let (l, marking_at_l) = legal_prefix_len(net, &order);
    let fired_in_prefix: std::collections::BTreeSet<TransitionId> =
        order[..l].iter().copied().collect();
    for t in &net.transitions {
        if !fired_in_prefix.contains(&t.id) {
            assert!(
                !firable(net, &marking_at_l, t.id, &index),
                "(I2) prefix not maximal: transition {:?} is firable at the stall point",
                t.id
            );
        }
    }

    // (I3) Greedy first-firable along the legal prefix.
    let mut marking = net.initial_marking.clone();
    let mut fired = vec![false; n];
    for (i, &chosen) in order.iter().take(l).enumerate() {
        // The chosen transition must be the FIRST un-fired firable one in
        // id order.
        let expected = (0..n)
            .map(|k| TransitionId(k as u32))
            .find(|&t| !fired[t.0 as usize] && firable(net, &marking, t, &index));
        assert_eq!(
            Some(chosen),
            expected,
            "(I3) step {i}: chosen {chosen:?} is not the first-firable-in-source-order"
        );
        net.fire_marking(chosen, &mut marking, &index)
            .expect("(I3) prefix step must fire");
        fired[chosen.0 as usize] = true;
    }
}

// --------------------------------------------------------------------
// (I1)-(I3) over the corpus + synthetic nets
// --------------------------------------------------------------------

/// The corpus exercised: a spread of single-worker (buffer-free) and
/// distributed/pipelined (buffered) nets so the invariant is pinned on
/// both the `L == N` (sound) regime and a constructed `L < N` (unsound)
/// regime (the synthetic overflow net below).
const CORPUS: &[(&str, &str)] = &[
    ("01-elementwise-add/prog.algo.nuc", "01-elementwise-add/schedules/naive.sched.nuc"),
    ("02-split-add/prog.algo.nuc", "02-split-add/schedules/split.sched.nuc"),
    ("03-reduction/prog.algo.nuc", "03-reduction/schedules/distributed.sched.nuc"),
    ("05-stencil/prog.algo.nuc", "05-stencil/schedules/distributed.sched.nuc"),
    ("07-matmul/prog.algo.nuc", "07-matmul/schedules/naive.sched.nuc"),
    ("07-matmul/prog.algo.nuc", "07-matmul/schedules/distributed.sched.nuc"),
    ("09-producer-consumer/prog.algo.nuc", "09-producer-consumer/schedules/pipelined.sched.nuc"),
    ("13-cnn-inference/prog.algo.nuc", "13-cnn-inference/schedules/pipeline_parallel.sched.nuc"),
];

#[test]
fn invariant_holds_over_corpus() {
    for (algo, sched) in CORPUS {
        let net = pipeline_to_net(algo, sched);
        assert_firing_order_invariant(&net);
    }
}

// --------------------------------------------------------------------
// (I5) source-order degeneration
// --------------------------------------------------------------------

#[test]
fn invariant_i5_source_order_degeneration_on_buffer_free_net() {
    // A buffer-free net (no nonzero buffer initial marking) — plain source
    // order is already legal, so the derived order must equal 0..N.
    let net = pipeline_to_net(
        "07-matmul/prog.algo.nuc",
        "07-matmul/schedules/naive.sched.nuc",
    );
    let order = derive_firing_order(&net);
    let source: Vec<TransitionId> = (0..net.transitions.len())
        .map(|i| TransitionId(i as u32))
        .collect();
    // Precondition: source order really is a legal firing sequence here.
    let (l, _) = legal_prefix_len(&net, &source);
    assert_eq!(
        l,
        net.transitions.len(),
        "precondition: source order must be legal on a buffer-free net"
    );
    assert_eq!(
        order, source,
        "(I5) on a net whose source order is legal, derive_firing_order must equal source order"
    );
}

// --------------------------------------------------------------------
// (I4) determinism
// --------------------------------------------------------------------

#[test]
fn invariant_i4_determinism() {
    for (algo, sched) in CORPUS {
        let net = pipeline_to_net(algo, sched);
        let a = derive_firing_order(&net);
        let b = derive_firing_order(&net);
        assert_eq!(a, b, "(I4) derive_firing_order must be deterministic");
    }
}

// --------------------------------------------------------------------
// (I2) on an UNSOUND net: L < N, and O[L] is the stall point
// --------------------------------------------------------------------

/// Build a net that overflows: a source pre-marked with 1, a cap-1 buffer,
/// and a transition that deposits 2 tokens into the buffer. The legal
/// prefix is empty (the only firable-from-source transition would
/// overflow), so `L == 0` and the invariant's maximal-prefix clause must
/// hold with no remaining firable transition.
fn overflow_net() -> Net {
    let mut net = Net::new();
    let src = net.add_place("src", cap(1), 1);
    let buf = net.add_place("buf", cap(1), 0);
    let sink = net.add_place("sink", cap(1), 0);
    let t = net.add_transition("overflow", None);
    net.add_arc(ArcKind::PtoT, src, t, 1);
    net.add_arc(ArcKind::TtoP, buf, t, 2); // deposits 2 into a cap-1 place
    net.add_arc(ArcKind::TtoP, sink, t, 1);
    net
}

#[test]
fn invariant_i2_holds_on_unsound_overflow_net() {
    let net = overflow_net();
    // The full invariant checker covers (I1)-(I3); on this net L < N and
    // it must still pass (maximal prefix with no remaining firable).
    assert_firing_order_invariant(&net);
    // Concretely: nothing is firable (the only transition overflows), so
    // the legal prefix is empty and the order is the single transition
    // appended as a stuck leftover.
    let order = derive_firing_order(&net);
    let (l, _) = legal_prefix_len(&net, &order);
    assert_eq!(l, 0, "no transition is firable, so L must be 0");
    assert_eq!(order.len(), 1, "the stuck transition is still appended (I1)");
}
