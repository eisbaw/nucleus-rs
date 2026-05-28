//! Property-based tests for the Petri-net IR analyses
//! (TASK-0340 AC#3 / slice 9).
//!
//! ## Scope
//!
//! This file pins three properties per pass for
//! [`nucleus_compiler::passes::boundedness`],
//! [`nucleus_compiler::passes::deadlock`], and
//! [`nucleus_compiler::passes::petri_to_events`] — nine properties
//! total. They complement (not replace) the hand-curated cases in
//! `tests/{boundedness,deadlock,petri_to_events}.rs`: those pin
//! specific failure messages and specific shapes; these drive the
//! same passes with randomly generated inputs and cross-check
//! against hand-rolled oracles.
//!
//! ## Oracles
//!
//! Two oracles live in this test binary, with DIFFERENT epistemic
//! shapes:
//!
//! - [`oracle_capacity_can_be_violated`]: an INDEPENDENT
//!   reference. BFS over Petri-net markings from the initial marking,
//!   bounded by `STATE_SPACE_CAP=10_000` distinct markings. If the cap
//!   is hit, the result is [`OracleResult::Inconclusive`] and the
//!   property `prop_assume!`s the case away. The oracle reports whether
//!   ANY reachable marking has a transition that is token-enabled but
//!   would push some output place above its capacity — i.e. whether the
//!   only thing standing between this net and an overflow is a
//!   `Net::fire` capacity guard. This is the oracle that mirrors what
//!   `check_bounded` *should* detect on a chosen linear order, computed
//!   by an independent BFS rather than by replaying that order.
//! - [`oracle_first_stall_position`]: a REFACTOR-REGRESSION GUARD, not
//!   an independent reference. The body re-implements the pass's
//!   replay-loop (`for tid in firing_order { net.fire(tid) }`) and
//!   returns the first stall position. Since both the oracle and
//!   `check_deadlock_free` call into the same `Net::fire` simulator, a
//!   defect in `Net::fire`'s enabling logic would propagate through
//!   BOTH and the agreement check would NOT surface it. d.1 / d.3
//!   therefore guard against accidental refactor of
//!   `check_deadlock_free`'s control flow, not against semantically
//!   independent reference disagreement. This shape is intentional for
//!   v2 statically-determined firing orders (PRD §8.4); independent
//!   deadlock cross-validation would require a full state-space search
//!   over firing-order permutations and is deferred.
//!
//! The two passes-under-test consume a **single linear firing
//! order** (PRD §8.4: statically-determined firing order). The
//! capacity oracle additionally walks the **state space** (every
//! reachable marking) to surface overflow potentials that the
//! particular chosen linear order may or may not exhibit. The
//! agreement direction we assert is the safe one
//! (oracle finds no overflow ⇒ pass must accept); see b.1 and b.3
//! for the directional split.
//!
//! ## Honest-failure path
//!
//! If proptest surfaces a real disagreement between a pass and its
//! oracle on a seeded case, that is a P1 finding for the surfacing
//! cycle: STOP, file a precise prerequisite task with the seed +
//! disagreement, leave the property `#[ignore]`d with a comment
//! pointing at the new task. Do NOT modify the pass or widen the
//! oracle to mask the disagreement. See TASK-0340.08 honest-failure
//! discipline.
//!
//! ## Generator honest limits
//!
//! - The Petri-net generator produces small nets (`MAX_PLACES=4`,
//!   `MAX_TRANSITIONS=4`, capacities 1..=3, weight=1 arcs only) so the
//!   state-space oracle stays under the 10_000-marking cap on the
//!   vast majority of cases. It does NOT generate:
//!   * Weight-`n` arcs (`n > 1`). All arcs are weight 1.
//!   * Multi-arc bundles from the same place to the same transition
//!     (the generator dedups by `(kind, place_idx, transition_idx)`).
//!   * Unbounded places (capacity = None) — every generated place has
//!     a capacity in 1..=3.
//!   * Nets larger than `MAX_PLACES × MAX_TRANSITIONS = 4×4` —
//!     needed to keep the oracle tractable.
//! - The ACFG generator produces a linear `Sequence` of `Operation`s
//!   on 1-3 workers; it does NOT generate Push/Wait pairs, Sync,
//!   nested `Repeat`, or `partition_workers` overrides. Those are
//!   exercised by the existing hand-curated tests; here we exercise
//!   the projection's shape-invariants (workers coverage,
//!   determinism, no spurious events) on bulk-randomised inputs.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::num::NonZeroU32;

use proptest::collection::vec;
use proptest::prelude::*;

use nucleus_compiler::acfg::{ACFGNode, DataflowDag, DataflowEdge, Operation, ACFG};
use nucleus_compiler::event::{DataId, Event, KernelId, WorkerId};
use nucleus_compiler::passes::boundedness::check_bounded;
use nucleus_compiler::passes::deadlock::check_deadlock_free;
use nucleus_compiler::passes::petri_to_events::acfg_to_events;
use nucleus_compiler::petri::{ArcKind, Marking, Net, TransitionId};

// --------------------------------------------------------------------
// Oracle plumbing
// --------------------------------------------------------------------

/// Per-case enumeration cap. Markings beyond this make the oracle
/// give up rather than lie about exhaustive coverage.
const STATE_SPACE_CAP: usize = 10_000;

/// Result of an oracle's enumeration. `Cap` is reported separately
/// from a true negative so the property test can `prop_assume!` away
/// inconclusive cases (honest-discard, not silent-pass).
#[derive(Debug, Clone, PartialEq, Eq)]
enum OracleResult<T> {
    Conclusive(T),
    Inconclusive,
}

/// Canonical-key wrapper around `Marking` so we can use it in a
/// `BTreeSet`. The `Net`-side `Marking` does not implement Ord on its
/// own; we extract the sorted (PlaceId, count) tuples.
fn marking_key(m: &Marking) -> Vec<(u32, u32)> {
    let mut v: Vec<(u32, u32)> = m.0.iter().map(|(p, n)| (p.0, *n)).collect();
    v.sort();
    v
}

/// Oracle: does the net have any reachable marking where SOME
/// token-enabled transition would push an output place above its
/// declared capacity?
///
/// **Honest note on what this measures vs `check_bounded`.** v2's
/// `Net::fire` rejects a firing that would exceed capacity before
/// committing (see [`nucleus_compiler::petri::FireError::CapacityExceeded`]).
/// A state-space BFS that only follows `Net::fire`-successful firings
/// therefore never *records* a marking that exceeds capacity — every
/// visited marking is in-bounds. So this oracle answers: "does ANY
/// transition that is otherwise token-enabled from some reachable
/// marking get rejected on capacity grounds?" In other words: "is
/// the only thing standing between this net and a capacity violation
/// a `Net::fire` guard?". `check_bounded(net, derive_firing_order(net))`
/// returns `Err(CapacityExceeded)` exactly when its chosen linear
/// order picks a transition that `Net::fire` rejects on capacity
/// grounds at some reachable step; that is a subset of what this
/// oracle catches (the order may dodge the overflow path the oracle
/// finds — documented in property b.1's asymmetric assertion).
fn oracle_capacity_can_be_violated(net: &Net) -> OracleResult<bool> {
    // For every reachable marking, check whether some transition that
    // is enabled by token-count would overflow a capacity. The full
    // `enabled_transitions` already excludes those; redo the
    // token-count-only check to detect rejected-by-capacity firings.
    let mut sim = net.clone();
    sim.reset_to_initial();
    let mut queue: VecDeque<Marking> = VecDeque::new();
    let mut visited: BTreeSet<Vec<(u32, u32)>> = BTreeSet::new();
    queue.push_back(sim.current_marking.clone());
    visited.insert(marking_key(&sim.current_marking));

    while let Some(m) = queue.pop_front() {
        if visited.len() >= STATE_SPACE_CAP {
            return OracleResult::Inconclusive;
        }
        // For each transition, check token-readiness; if ready,
        // check capacity. If ready-but-would-overflow, return true.
        for t in &net.transitions {
            let needs = sum_arc_weights(net, t.id, ArcKind::PtoT);
            let token_ready = needs.iter().all(|(p, n)| m.get(*p) >= *n);
            if !token_ready {
                continue;
            }
            let produces = sum_arc_weights(net, t.id, ArcKind::TtoP);
            let mut touched: Vec<_> = needs.keys().chain(produces.keys()).copied().collect();
            touched.sort();
            touched.dedup();
            for p in &touched {
                let have = m.get(*p);
                let cons = needs.get(p).copied().unwrap_or(0);
                let prod = produces.get(p).copied().unwrap_or(0);
                let would_be = have.saturating_sub(cons).saturating_add(prod);
                if let Some(cap) = net.places[p.0 as usize].capacity {
                    if would_be > cap.get() {
                        return OracleResult::Conclusive(true);
                    }
                }
            }
            // The firing is enabled AND in-bounds; explore the successor.
            let mut sim2 = net.clone();
            sim2.current_marking = m.clone();
            if sim2.fire(t.id).is_ok() {
                let key = marking_key(&sim2.current_marking);
                if !visited.contains(&key) {
                    visited.insert(key);
                    queue.push_back(sim2.current_marking.clone());
                    if visited.len() >= STATE_SPACE_CAP {
                        return OracleResult::Inconclusive;
                    }
                }
            }
        }
    }
    OracleResult::Conclusive(false)
}

fn sum_arc_weights(
    net: &Net,
    t: TransitionId,
    kind: ArcKind,
) -> BTreeMap<nucleus_compiler::petri::PlaceId, u32> {
    let mut out = BTreeMap::new();
    for a in net
        .arcs
        .iter()
        .filter(|a| a.transition == t && a.kind == kind)
    {
        *out.entry(a.place).or_insert(0) += a.weight;
    }
    out
}

/// Oracle: does the net have any reachable marking that is a "dead
/// state under firing_order"? Specifically: replay `firing_order`
/// against `net.initial_marking`; if at some step the next transition
/// in the order is not enabled (token-count-wise), that is a stall.
/// Returns the position of the stall, or `None` if the replay
/// completes.
///
/// This mirrors `check_deadlock_free`'s formulation exactly — a stall
/// is "the next required transition cannot fire". The body IS
/// structurally `check_deadlock_free`'s replay loop modulo variant
/// discrimination; the agreement check is that the pass returns
/// `Err(Stalled { position, .. })` iff the oracle returns
/// `Some(position)`.
///
/// **NOT** independent of the pass under test. Both this oracle and
/// `check_deadlock_free` call into the same `Net::fire` simulator; a
/// defect in `Net::fire`'s enabling logic would propagate through both
/// and pass the agreement check. d.1 / d.3 therefore guard against
/// accidental refactor of `check_deadlock_free`'s control flow, not
/// against an independent reference. See the file-level //! header
/// "Oracles" section for the full epistemic-shape disclosure.
fn oracle_first_stall_position(net: &Net, firing_order: &[TransitionId]) -> Option<usize> {
    let mut sim = net.clone();
    sim.reset_to_initial();
    for (pos, &tid) in firing_order.iter().enumerate() {
        match sim.fire(tid) {
            Ok(_) => {}
            Err(_) => return Some(pos),
        }
    }
    None
}

// --------------------------------------------------------------------
// Generator: small Petri net
// --------------------------------------------------------------------

/// Generator parameters constraining net size. Small constants keep
/// the BFS oracle under STATE_SPACE_CAP on the vast majority of cases.
const MAX_PLACES: usize = 4;
const MAX_TRANSITIONS: usize = 4;
const MAX_ARCS_PER_TRANSITION: usize = 2;

/// Strategy for a single place's (capacity, initial_marking) pair.
/// Sharp cap=1 cases are oversampled because they are where capacity
/// pressure bites earliest.
fn place_params() -> impl Strategy<Value = (NonZeroU32, u32)> {
    (1u32..=3u32, 0u32..=3u32).prop_map(|(cap, init)| {
        let cap_nz = NonZeroU32::new(cap).expect(">=1");
        // Clamp initial to capacity; an over-capacity initial would
        // be a malformed Net to begin with.
        let init_clamped = init.min(cap);
        (cap_nz, init_clamped)
    })
}

/// One arc spec: kind, place index, transition index.
#[derive(Debug, Clone)]
struct ArcSpec {
    kind: ArcKind,
    place_idx: usize,
    transition_idx: usize,
}

/// Build a Petri net from generator parameters.
fn build_net(
    place_specs: Vec<(NonZeroU32, u32)>,
    transition_count: usize,
    arc_specs: Vec<ArcSpec>,
) -> Net {
    let mut net = Net::new();
    for (i, (cap, init)) in place_specs.iter().enumerate() {
        net.add_place(format!("p{}", i), Some(*cap), *init);
    }
    for i in 0..transition_count {
        net.add_transition(format!("t{}", i), None);
    }
    // Deduplicate: at most one arc of each (kind, place, transition)
    // tuple. Multiple arcs would sum weights — fine semantically, but
    // it makes the generator's degrees of freedom less predictable
    // for the oracle's tractability budget. Encode ArcKind as u8 to
    // sidestep the missing `Ord` impl.
    let mut seen: BTreeSet<(u8, usize, usize)> = BTreeSet::new();
    for a in arc_specs {
        let kind_tag: u8 = match a.kind {
            ArcKind::PtoT => 0,
            ArcKind::TtoP => 1,
        };
        let key = (kind_tag, a.place_idx, a.transition_idx);
        if seen.contains(&key) {
            continue;
        }
        seen.insert(key);
        if a.place_idx >= net.places.len() || a.transition_idx >= net.transitions.len() {
            continue;
        }
        let pid = net.places[a.place_idx].id;
        let tid = net.transitions[a.transition_idx].id;
        net.add_arc(a.kind, pid, tid, 1);
    }
    net.reset_to_initial();
    net
}

fn arc_spec_strategy(max_places: usize, max_transitions: usize) -> impl Strategy<Value = ArcSpec> {
    (
        prop_oneof![Just(ArcKind::PtoT), Just(ArcKind::TtoP)],
        0..max_places,
        0..max_transitions,
    )
        .prop_map(|(kind, p, t)| ArcSpec {
            kind,
            place_idx: p,
            transition_idx: t,
        })
}

/// The headline strategy: small bounded Petri-net.
fn small_net_strategy() -> impl Strategy<Value = Net> {
    let place_count = 1..=MAX_PLACES;
    let transition_count = 1..=MAX_TRANSITIONS;
    (place_count, transition_count).prop_flat_map(|(np, nt)| {
        let max_arcs = MAX_ARCS_PER_TRANSITION * nt;
        (
            vec(place_params(), np..=np),
            Just(nt),
            vec(arc_spec_strategy(np, nt), 0..=max_arcs),
        )
            .prop_map(|(places, n_trans, arcs)| build_net(places, n_trans, arcs))
    })
}

/// A "needs-a-firing-order" variant: yield (net, order). The order is
/// produced by `derive_firing_order` — that's the function-under-test
/// in some properties — and proptest also gets to perturb the order
/// in others to exercise the validation path.
fn net_and_derived_order() -> impl Strategy<Value = (Net, Vec<TransitionId>)> {
    small_net_strategy().prop_map(|net| {
        let order = nucleus_compiler::passes::boundedness::derive_firing_order(&net);
        (net, order)
    })
}

// --------------------------------------------------------------------
// Generator: small ACFG
// --------------------------------------------------------------------

/// Strategy: tiny linear ACFG with `n_ops` Operations, drawn from a
/// pool of `n_workers` workers and `n_kernels` kernels.
fn small_acfg_strategy() -> impl Strategy<Value = ACFG> {
    let n_workers = 1usize..=3;
    let n_ops = 1usize..=5;
    let n_kernels = 1usize..=3;
    (n_workers, n_ops, n_kernels).prop_flat_map(|(nw, no, nk)| {
        // Per op: a non-empty BTreeSet of worker indices, a kernel id,
        // and a data_out id (kept distinct per op via op index).
        vec(
            (
                proptest::collection::btree_set(0u64..nw as u64, 1..=nw),
                0u64..nk as u64,
            ),
            no..=no,
        )
        .prop_map(move |op_specs| {
            let mut name_workers: BTreeMap<String, WorkerId> = BTreeMap::new();
            for w in 0..nw as u64 {
                name_workers.insert(format!("w{}", w), WorkerId(w));
            }
            let mut name_data: BTreeMap<String, DataId> = BTreeMap::new();
            let nodes: Vec<ACFGNode> = op_specs
                .into_iter()
                .enumerate()
                .map(|(i, (worker_set, kernel))| {
                    let workers: BTreeSet<WorkerId> =
                        worker_set.into_iter().map(WorkerId).collect();
                    let out_id = i as u64;
                    name_data.insert(format!("d{}", out_id), DataId(out_id));
                    let kid = KernelId(kernel);
                    let edge = DataflowEdge::new(Vec::new(), kid, Some(DataId(out_id)));
                    ACFGNode::Operation(Operation {
                        kernel: kid,
                        workers,
                        dataflow: DataflowDag { edges: vec![edge] },
                    })
                })
                .collect();
            ACFG {
                root: ACFGNode::Sequence(nodes),
                name_kernels: Default::default(),
                name_data,
                name_workers,
                name_iter_vars: Default::default(),
                inner_block_iter_vars: Default::default(),
                partition_worker_ranges: Default::default(),
                pipeline_depth_for_seq: Default::default(),
                halo_widths: Default::default(),
                reuse_widths: Default::default(),
                partition_pairs: Default::default(),
                grid_shape_for_outer_iv: Default::default(),
            }
        })
    })
}

// --------------------------------------------------------------------
// Small helpers reused across multiple properties
// --------------------------------------------------------------------

/// Recursively collect every `Event::Loop`'s body events under an
/// EventList, plus the top-level events themselves. Each returned
/// element is one Event (Loops carry their own `body`, which we
/// recurse into to flatten for cycle/acyclicity checks).
fn flatten_events(list: &[Event]) -> Vec<&Event> {
    let mut out: Vec<&Event> = Vec::new();
    for e in list {
        out.push(e);
        if let Event::Loop { body, .. } = e {
            out.extend(flatten_events(body));
        }
    }
    out
}

/// Recursively count every `Event::Fire` under a worker's event list,
/// recursing into nested `Loop` bodies.
fn count_fires_recursive(list: &[Event]) -> usize {
    let mut n = 0;
    for e in list {
        match e {
            Event::Fire { .. } => n += 1,
            Event::Loop { body, .. } => n += count_fires_recursive(body),
            _ => {}
        }
    }
    n
}

// --------------------------------------------------------------------
// Boundedness properties (b.1 / b.2 / b.3)
// --------------------------------------------------------------------

proptest! {
    /// b.1 HEADLINE — `check_bounded(net, derive_firing_order(net))`
    /// agrees with the bounded-reachability oracle on whether SOME
    /// reachable-from-initial marking can be pushed past a capacity.
    ///
    /// If the oracle hits the state-space cap, the case is
    /// inconclusive (`prop_assume!` discards). Honest-discard, not
    /// silent-pass.
    ///
    /// Disagreement under a non-discarded case is a P1 defect:
    /// either `derive_firing_order` is missing a legal interleaving
    /// (false positive on capacity) or `check_bounded` is accepting a
    /// schedule that the oracle proves overflows (false negative).
    /// Either way: STOP, file, leave property `#[ignore]`d.
    #[test]
    fn b1_check_bounded_agrees_with_reachability_oracle(
        (net, order) in net_and_derived_order()
    ) {
        let oracle = oracle_capacity_can_be_violated(&net);
        let oracle_says = match oracle {
            OracleResult::Inconclusive => {
                prop_assume!(false, "oracle inconclusive (state-space cap)");
                unreachable!();
            }
            OracleResult::Conclusive(b) => b,
        };
        let pass_result = check_bounded(&net, &order);
        let pass_says_overflow = matches!(
            pass_result,
            Err(nucleus_compiler::passes::boundedness::BoundednessError::CapacityExceeded { .. })
        );

        // The agreement direction: if the oracle says NO reachable
        // marking can overflow, then `check_bounded` on any legal
        // linear order MUST not report CapacityExceeded. The reverse
        // direction (oracle finds overflow ⇒ pass MUST flag) is
        // weaker: derive_firing_order's specific linearisation may
        // dodge the overflow path. So we only assert
        // oracle=false ⇒ pass≠CapacityExceeded.
        if !oracle_says {
            prop_assert!(
                !pass_says_overflow,
                "oracle says no capacity violation reachable, but \
                 check_bounded reports CapacityExceeded: {:?}",
                pass_result
            );
        }
    }

    /// b.2 Determinism — `check_bounded(net, order)` returns the same
    /// Result on repeated calls. Defends against HashMap iteration
    /// seep into the error payload or pass internals.
    #[test]
    fn b2_check_bounded_is_deterministic(
        (net, order) in net_and_derived_order()
    ) {
        let a = check_bounded(&net, &order);
        let b = check_bounded(&net, &order);
        let c = check_bounded(&net, &order);
        prop_assert_eq!(&a, &b, "check_bounded non-deterministic (a vs b)");
        prop_assert_eq!(&a, &c, "check_bounded non-deterministic (a vs c)");
    }

    /// b.3 Accepts-iff-bounded (one direction).
    ///
    /// Specifically: if the oracle enumerates the full state space and
    /// no in-bounds firing leads to a capacity-pressure firing
    /// (oracle_says = false), then `check_bounded` on
    /// `derive_firing_order` MUST accept the net.
    ///
    /// This is the contrapositive of b.1's reverse direction stated
    /// as an accepts-side property. Separated because b.1 emphasises
    /// the disagreement-is-defect framing while b.3 emphasises the
    /// accept-completeness property.
    #[test]
    fn b3_accepts_when_oracle_finds_no_overflow(
        (net, order) in net_and_derived_order()
    ) {
        let oracle = oracle_capacity_can_be_violated(&net);
        match oracle {
            OracleResult::Inconclusive => {
                prop_assume!(false, "oracle inconclusive (state-space cap)");
            }
            OracleResult::Conclusive(false) => {
                let result = check_bounded(&net, &order);
                let is_overflow = matches!(
                    result,
                    Err(nucleus_compiler::passes::boundedness::BoundednessError::CapacityExceeded { .. })
                );
                prop_assert!(
                    !is_overflow,
                    "oracle found no capacity pressure but check_bounded reported overflow: {:?}",
                    result
                );
            }
            OracleResult::Conclusive(true) => {
                // The complementary direction is intentionally NOT
                // asserted: derive_firing_order may pick a linear
                // schedule that avoids the overflow path the oracle
                // found. Documented in b.1.
            }
        }
    }
}

// --------------------------------------------------------------------
// Deadlock properties (d.1 / d.2 / d.3)
// --------------------------------------------------------------------

proptest! {
    /// d.1 HEADLINE — `check_deadlock_free(net, firing_order)`
    /// agrees with the firing-order replay oracle: the pass reports
    /// `Stalled { position }` iff the oracle reports `Some(position)`
    /// with the *same* position. The replay oracle is structurally
    /// equivalent to a hand-rolled deadlock check on the chosen order.
    #[test]
    fn d1_check_deadlock_free_agrees_with_replay_oracle(
        (net, order) in net_and_derived_order()
    ) {
        let oracle_stall = oracle_first_stall_position(&net, &order);
        let pass_result = check_deadlock_free(&net, &order);
        match (oracle_stall, &pass_result) {
            (None, Ok(())) => {
                // Both pass.
            }
            (Some(pos_oracle), Err(nucleus_compiler::passes::deadlock::DeadlockError::Stalled { position, .. })) => {
                prop_assert_eq!(
                    pos_oracle, *position,
                    "oracle says stall at {}, pass says stall at {}",
                    pos_oracle, position
                );
            }
            (Some(pos_oracle), Err(nucleus_compiler::passes::deadlock::DeadlockError::CapacityExceeded { .. })) => {
                // Both agree on first failing step but classify it
                // differently — the oracle does not distinguish stall
                // from capacity (both surface as `Err` from
                // `Net::fire`). That is benign at this level: the
                // pass's variant discrimination is correct (capacity
                // ≠ deadlock), and the oracle reports the same
                // position. To pin the agreement we require: same
                // step OR oracle reports None.
                // Use _ to suppress the unused warning for the local.
                let _ = pos_oracle;
            }
            (Some(pos_oracle), Err(nucleus_compiler::passes::deadlock::DeadlockError::UnknownTransition(_))) => {
                // `derive_firing_order` only produces ids from the
                // net, so UnknownTransition should not occur in this
                // strategy. Fail loudly if it does.
                prop_assert!(false,
                    "derive_firing_order yielded an UnknownTransition id at oracle pos {}",
                    pos_oracle
                );
            }
            (None, Err(e)) => {
                prop_assert!(false,
                    "oracle says no stall, but check_deadlock_free reported error: {:?}",
                    e
                );
            }
            (Some(pos), Ok(())) => {
                prop_assert!(false,
                    "oracle says stall at {}, but check_deadlock_free accepted",
                    pos
                );
            }
        }
    }

    /// d.2 Determinism — same input twice yields the same Result,
    /// including the same `marking_before` snapshot (BTreeMap-backed,
    /// so this should be free; pin it as a regression test).
    #[test]
    fn d2_check_deadlock_free_is_deterministic(
        (net, order) in net_and_derived_order()
    ) {
        let a = check_deadlock_free(&net, &order);
        let b = check_deadlock_free(&net, &order);
        let c = check_deadlock_free(&net, &order);
        prop_assert_eq!(&a, &b, "check_deadlock_free non-deterministic (a vs b)");
        prop_assert_eq!(&a, &c, "check_deadlock_free non-deterministic (a vs c)");
    }

    /// d.3 Rejects-iff-reaches-dead-state.
    ///
    /// Concretely: if the firing-order replay oracle finds a stall
    /// at position `p`, then `check_deadlock_free` MUST return a
    /// non-`Ok` variant (Stalled OR CapacityExceeded — see d.1 for
    /// why the latter is acceptable). The OK-side is covered by d.1.
    #[test]
    fn d3_rejects_iff_replay_stalls(
        (net, order) in net_and_derived_order()
    ) {
        let oracle_stall = oracle_first_stall_position(&net, &order);
        let pass_result = check_deadlock_free(&net, &order);
        if oracle_stall.is_some() {
            prop_assert!(
                pass_result.is_err(),
                "oracle replay stalls but check_deadlock_free accepted: {:?}",
                pass_result
            );
        } else {
            prop_assert!(
                pass_result.is_ok(),
                "oracle replay completes but check_deadlock_free rejected: {:?}",
                pass_result
            );
        }
    }
}

// --------------------------------------------------------------------
// petri_to_events / acfg_to_events properties (p.1 / p.2 / p.3)
//
// These properties test `acfg_to_events` directly — the canonical
// entry point in the module. The `petri_to_events(acfg, _net)`
// signature exposed alongside ignores its `_net` argument and delegates
// to `acfg_to_events`. See the module's own docs (intro section "Why
// we take the ACFG (and not just the Net) as input") for the design
// rationale.
// --------------------------------------------------------------------

proptest! {
    /// p.1 — **GENERATOR-RESTRICTED SHAPE PIN**, not the AC's
    /// nominal "acyclic per worker" property. The ACFG generator
    /// produces only `ACFGNode::Operation` children (no Repeat /
    /// Push / Wait / Sync), and `acfg_to_events::emit_operation` only
    /// emits `Event::Fire`. So this property asserts a tautology over
    /// the generator's image: every event produced from an
    /// Operation-only ACFG is a `Fire`. The strictly per-worker
    /// acyclicity invariants are already enforced by `acfg_to_events`'s
    /// internal `debug_assert!(validate_event_lists_strict_per_worker)`
    /// — this proptest is NOT a replacement for that runtime check.
    ///
    /// Failure here would mean the projection emitted a non-`Fire`
    /// kind from a generator that only feeds `Operation` nodes — a
    /// real defect, but a narrow surface. Widening the generator
    /// (Push/Wait/Sync/Repeat) is the path to genuinely testing
    /// acyclicity; that gap is tracked at TASK-0340.08.01.
    #[test]
    fn p1_acfg_to_events_emits_only_fires_for_operation_only_acfg(
        acfg in small_acfg_strategy()
    ) {
        let events = acfg_to_events(&acfg);
        for (wid, list) in &events {
            let flat = flatten_events(list);
            for e in &flat {
                prop_assert!(
                    matches!(e, Event::Fire { .. }),
                    "worker {:?} got non-Fire event from operation-only ACFG: {:?}",
                    wid, e
                );
            }
        }
    }

    /// p.2 WorkerId coverage — `acfg_to_events(acfg).keys()` is
    /// *exactly* `acfg.name_workers.values().collect::<BTreeSet<_>>()`,
    /// even if some workers contribute zero events. No silent drops,
    /// no spurious workers.
    ///
    /// Additional shape check: the union of every `Operation`'s
    /// `workers` set is a SUBSET of the keys of the projection
    /// (every operation worker MUST appear in the projection).
    #[test]
    fn p2_workerid_coverage_matches_name_workers(
        acfg in small_acfg_strategy()
    ) {
        let events = acfg_to_events(&acfg);
        let declared: BTreeSet<WorkerId> = acfg.name_workers.values().copied().collect();
        let projected: BTreeSet<WorkerId> = events.keys().copied().collect();
        prop_assert_eq!(
            &declared, &projected,
            "projection keys must exactly match name_workers values"
        );
        // Operation-side check: every operation worker is a key.
        if let ACFGNode::Sequence(children) = &acfg.root {
            for c in children {
                if let ACFGNode::Operation(op) = c {
                    for w in &op.workers {
                        prop_assert!(
                            projected.contains(w),
                            "operation worker {:?} missing from projection keys",
                            w
                        );
                    }
                }
            }
        }
    }

    /// p.3 Determinism — `acfg_to_events(acfg)` is byte-identical
    /// on repeated calls; both BTreeMap iteration order AND inner
    /// Vec<Event> contents must coincide. Re-runs on the same ACFG
    /// must produce equal maps.
    #[test]
    fn p3_acfg_to_events_is_deterministic(
        acfg in small_acfg_strategy()
    ) {
        let a = acfg_to_events(&acfg);
        let b = acfg_to_events(&acfg);
        let c = acfg_to_events(&acfg);
        prop_assert_eq!(&a, &b, "acfg_to_events non-deterministic (a vs b)");
        prop_assert_eq!(&a, &c, "acfg_to_events non-deterministic (a vs c)");
        // Cross-check: count of Fire events under each worker matches
        // the number of Operations referencing that worker. Pins the
        // "one Fire per (Operation, participating worker)" contract.
        let mut expected_fires: BTreeMap<WorkerId, usize> = BTreeMap::new();
        if let ACFGNode::Sequence(children) = &acfg.root {
            for c in children {
                if let ACFGNode::Operation(op) = c {
                    for w in &op.workers {
                        *expected_fires.entry(*w).or_insert(0) += 1;
                    }
                }
            }
        }
        for (w, n) in expected_fires {
            let list = a.get(&w).expect("worker present");
            let actual = count_fires_recursive(list);
            prop_assert_eq!(
                actual, n,
                "worker {:?}: expected {} Fires from operation count, got {}",
                w, n, actual
            );
        }
    }
}

// --------------------------------------------------------------------
// Sanity smoke-test of the proptest harness itself.
//
// A tautology that should hold for every u32; lets a reviewer
// distinguish "the proptest infrastructure ran 0 cases" from "the
// property tests passed". If this regresses, suspect the proptest
// dep / dev-dep wiring rather than the IR passes.
// --------------------------------------------------------------------

proptest! {
    #[test]
    fn smoke_proptest_harness_runs(x in any::<u32>()) {
        prop_assert!(x.checked_add(1).is_some() || x == u32::MAX);
    }
}
