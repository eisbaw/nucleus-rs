//! Symbolic soundness for COMMUNICATING nets — the steady-state
//! occupancy argument over the looped Push/Wait structure
//! (TASK-0455.01, the keystone of the production push; PRD §8.1 / §8.4).
//!
//! ## Why this exists (the keystone wall)
//!
//! [`super::net_soundness_symbolic`] (TASK-0453.04) lifted the
//! linear-in-firings cost of the expanded soundness gate for
//! **buffer-free** nets only — single-worker programs and multi-worker
//! programs with no cross-worker transfer. The moment a schedule
//! distributes work and *communicates*, the ACFG carries a buffer place
//! per [`SeqTag`](crate::event::SeqTag) and the gate fell back to the
//! expanded single-order replay, whose net carries ~2 nodes per kernel
//! firing. At `07-matmul` distributed, N=512, that net is ~25 GB of RSS
//! (measured, TASK-0453 cycle 6) — it OOMs before the gate even runs,
//! blocking exactly the decompositions production needs.
//!
//! This module decides soundness for a **decidable subclass of
//! communicating nets** directly from the ROLLED ACFG — whose size is a
//! function of the program text and worker count, NOT of the iteration
//! counts — so a distributed schedule whose loops run N times is decided
//! in `O(ACFG nodes)`, independent of N. No expanded net is built.
//!
//! ## The subclass it proves: single-shot matched Push/Wait pairs
//!
//! [`acfg_to_net`](crate::passes::acfg_to_petri::acfg_to_net) emits, per
//! [`SeqTag`], **one buffer place** `buf_<data>_seq<n>` of capacity
//! `C = policy.buffer` (`>= 1`) and initial marking `P =
//! pipeline_depth_for_seq[seq]` (`0` unless the pair sits in a
//! `pipeline=D` loop). A `Push` deposits one token into it (a `TtoP`
//! arc); the matching `Wait` consumes one (a `PtoT` arc). The buffer
//! place is the ONLY structure that can make a net unsound — every
//! control place is capacity-1, single-producer, single-consumer (see
//! the buffer-free theorem in [`super::net_soundness_symbolic`]).
//!
//! Each `SeqTag` has **exactly one `Push` node and one `Wait` node in the
//! rolled ACFG** (the transfer-injection pass emits matched pairs; this
//! is an empirically-confirmed structural fact over the whole corpus, and
//! a violation is conservatively classified [`SymbolicSoundness::NeedsExpansion`](super::net_soundness_symbolic::SymbolicSoundness::NeedsExpansion) below, not
//! assumed). How many *transitions* that buffer place ends up with in the
//! EXPANDED net depends solely on how many enclosing `Repeat` loops the
//! Push/Wait nodes sit inside, because
//! [`acfg_to_net`](crate::passes::acfg_to_petri) unrolls each `Repeat`
//! over its full range:
//!
//! - **Loop-depth 0** (the Push/Wait nodes are NOT nested in any
//!   `Repeat`): the buffer place gets exactly ONE `Push` transition and
//!   ONE `Wait` transition. THIS is the single-shot subclass this module
//!   proves.
//! - **Loop-depth `>= 1`**: the buffer place is shared across the
//!   unrolled iterations and gets N `Push`/N `Wait` transitions. Whether
//!   its peak occupancy stays within `C` then depends on the
//!   per-iteration drain interleaving under the greedy firing order — a
//!   genuine steady-state argument this first landing does NOT attempt.
//!   Classified [`SymbolicSoundness::NeedsExpansion`](super::net_soundness_symbolic::SymbolicSoundness::NeedsExpansion) (loud fallback), NOT optimistically
//!   accepted.
//!
//! ### The single-shot soundness theorem
//!
//! > **Theorem (single-shot communicating net).** Let an ACFG be such
//! > that every `SeqTag` has exactly one `Push` and one `Wait` node, both
//! > at loop-depth 0 (no enclosing `Repeat`), with `Push` textually
//! > before `Wait` in source order, capacity `C >= 1`, and pipeline
//! > pre-mark `P = 0`. Then the net
//! > [`acfg_to_net`](crate::passes::acfg_to_petri::acfg_to_net) produces
//! > is bounded, deadlock-free and conflict-free — i.e.
//! > [`check_net_sound`](super::net_soundness::check_net_sound) returns
//! > `Ok(())`.
//!
//! **Proof.** Each buffer place `b` then has exactly one producer
//! transition (its single `Push`, deposit weight 1), one consumer
//! transition (its single `Wait`, consume weight 1), capacity `C >= 1`,
//! and initial marking `0`. Consider the three rejection modes the gate
//! checks, against the derived firing order (whose stated invariant
//! `(I1)-(I3)` is pinned in `tests/firing_order_invariant.rs`):
//!
//! - **Boundedness.** `b`'s occupancy is `(#Push fired) - (#Wait fired)`.
//!   There is exactly one `Push`, so at most one token is ever deposited;
//!   occupancy is `0` or `1`, never above `C >= 1`. No overflow.
//! - **Deadlock (no stall).** The single `Wait`'s only buffer input is
//!   `b`; by the firing-order invariant `(I3)`, a transition is only
//!   placed in the legal prefix when *firable*, and the `Wait` becomes
//!   firable exactly once its `Push` has deposited `b`'s token. The
//!   `Push` is itself reachable (it sits on its producer worker's control
//!   chain, discharged in id order), so the order fires `Push` then
//!   `Wait`; the `Wait` never stalls. (`Push`-before-`Wait` in source
//!   order is required here only as a conservative guard — the buffer
//!   data-dependency forces that order regardless — but a `Wait`-before-
//!   `Push` *source* layout is conservatively rejected to keep the
//!   theorem's hypotheses literal.)
//! - **Conflict-freedom (PRD §8.4(a)).** `b` has exactly one consumer
//!   transition (its single `Wait`), so it can never co-enable two
//!   consumers. Every control place has one consumer by construction. No
//!   free-choice conflict at any reachable marking. ∎
//!
//! The argument is iteration-count independent: it inspects the rolled
//! ACFG's Push/Wait nodes once each, regardless of how many times any
//! enclosing loop runs (those loops do not enclose the single-shot pairs,
//! by hypothesis — a Push/Wait at loop-depth `>= 1` is what excludes a
//! seq from the subclass).
//!
//! ## Soundness discipline (non-negotiable — thesis `sec:fw-quant`)
//!
//! This is a fast path, NEVER a weakening. It returns
//! [`SymbolicSoundness::ProvenSound`] ONLY when *every* seq in the ACFG
//! satisfies the theorem's hypotheses; ANY seq outside the subclass
//! (loop-nested Push/Wait, a pre-mark `P > 0`, a non-unit Push/Wait
//! count, a Wait-before-Push source layout, a saturating capacity) routes
//! the WHOLE ACFG to [`SymbolicSoundness::NeedsExpansion`] with a
//! `&'static str` reason for the `NUC_TRACE` advisory — the unchanged
//! expanded gate then decides. There is **no silent optimistic accept**:
//! an imprecise buffer analysis that accepted a net the expanded replay
//! would reject is exactly what the gate exists to forbid, so anything
//! the steady-state argument does not literally cover falls back loudly.
//! The A/B equivalence harness (`tests/symbolic_comm_ab.rs`) pins
//! `symbolic verdict == expanded verdict` on every corpus net and the
//! negative unit nets.
//!
//! ## Honest residual (what still falls back, tracked as follow-ups)
//!
//! - **Loop-carried transfers** (Push/Wait at loop-depth `>= 1`, `P = 0`)
//!   — e.g. `16-jacobi` distributed. Empirically the buffer drains each
//!   iteration (peak 1), but proving it needs the cross-iteration
//!   drainage / capacity-forced-interleaving argument; deferred.
//! - **Pipelined transfers** (`P = pipeline_depth > 0`) — e.g.
//!   `09-producer-consumer`/`13-cnn-inference` pipelined. Peak `= C`,
//!   sound, but the steady-state pre-mark argument is the most delicate
//!   and is deferred.
//!
//! Both are classified [`SymbolicSoundness::NeedsExpansion`](super::net_soundness_symbolic::SymbolicSoundness::NeedsExpansion) today; they are the natural
//! next subclasses to add WITH PROOF (each with its own A/B pin).

use std::collections::BTreeMap;

use crate::acfg::{ACFGNode, XferRole, ACFG};
use crate::event::SeqTag;

use super::net_soundness_symbolic::SymbolicSoundness;

/// Per-seq structural facts collected from one walk of the rolled ACFG.
///
/// Everything the single-shot theorem's hypotheses need is captured here
/// so the decision is a pure function of these aggregates — no second
/// walk, no expanded net.
#[derive(Debug, Default, Clone)]
struct SeqFacts {
    /// Number of `Push` nodes carrying this seq in the rolled ACFG.
    push_count: usize,
    /// Number of `Wait` nodes carrying this seq in the rolled ACFG.
    wait_count: usize,
    /// Maximum enclosing-`Repeat` nesting depth of any `Push` node for
    /// this seq. `0` = not inside any loop (single-shot candidate).
    max_push_depth: usize,
    /// Maximum enclosing-`Repeat` nesting depth of any `Wait` node.
    max_wait_depth: usize,
    /// True once a `Push` for this seq has been seen earlier in the
    /// source-order walk than this point.
    push_seen: bool,
    /// True iff a `Wait` for this seq appeared in source order BEFORE any
    /// `Push` for it (a Wait-before-Push layout — conservatively
    /// rejected).
    wait_before_push: bool,
    /// The transfer's buffer capacity `C` (`policy.buffer`, `>= 1`).
    /// `None` until the first Push/Wait for the seq is seen.
    capacity: Option<u64>,
}

/// Walk the rolled ACFG once, accumulating [`SeqFacts`] per [`SeqTag`].
///
/// A `Repeat` body is descended ONCE at `depth + 1` (not per iteration):
/// the loop-nesting depth of a Push/Wait node is a structural property of
/// the program text, independent of the iteration count. This is what
/// keeps the analysis `O(ACFG nodes)` and iteration-count independent.
fn collect_seq_facts(node: &ACFGNode, depth: usize, facts: &mut BTreeMap<SeqTag, SeqFacts>) {
    match node {
        ACFGNode::Xfer(x) => {
            let f = facts.entry(x.seq).or_default();
            // Capacity is a per-pair policy field; both endpoints are
            // built from one policy today (transfer_inject clones the
            // Wait's policy into the Push), but the expanded gate sizes
            // the place from the FIRST-seen endpoint — so take the MIN
            // across endpoints rather than last-seen, making the
            // theorem's C >= 1 hypothesis literal even if the endpoints
            // ever diverged (wave-6 review P3.1).
            f.capacity = Some(match f.capacity {
                Some(prev) => prev.min(x.policy.buffer),
                None => x.policy.buffer,
            });
            match x.role {
                XferRole::Push => {
                    f.push_count += 1;
                    f.max_push_depth = f.max_push_depth.max(depth);
                    f.push_seen = true;
                }
                XferRole::Wait => {
                    f.wait_count += 1;
                    f.max_wait_depth = f.max_wait_depth.max(depth);
                    if !f.push_seen {
                        f.wait_before_push = true;
                    }
                }
            }
        }
        ACFGNode::Operation(_) | ACFGNode::Sync(_) => {}
        ACFGNode::Sequence(children) => {
            for c in children {
                collect_seq_facts(c, depth, facts);
            }
        }
        ACFGNode::Repeat { body, .. } => collect_seq_facts(body, depth + 1, facts),
    }
}

/// Decide whether a single seq satisfies the single-shot theorem's
/// hypotheses. Returns `Ok(())` if it is in the proven subclass, or
/// `Err(reason)` naming WHY it is outside (for the loud fallback).
///
/// `pipeline_p` is the seq's `pipeline_depth_for_seq` pre-mark (`0` if
/// absent).
fn classify_seq(f: &SeqFacts, pipeline_p: u64) -> Result<(), &'static str> {
    // Exactly one Push and one Wait. A seq with a different count is
    // outside the single-shot shape the theorem reasons about.
    if f.push_count != 1 || f.wait_count != 1 {
        return Err(
            "a buffer seq has a non-unit Push/Wait count (not the single-shot matched-pair \
             shape the steady-state theorem covers)",
        );
    }
    // Both endpoints must be at loop-depth 0 (not nested in any Repeat).
    // A loop-nested pair shares its buffer place across unrolled
    // iterations; its peak occupancy depends on the per-iteration drain
    // interleaving, which the single-shot theorem does not cover.
    if f.max_push_depth != 0 || f.max_wait_depth != 0 {
        return Err(
            "a buffer seq's Push/Wait is loop-nested (shared across unrolled iterations); its \
             steady-state peak occupancy is not covered by the single-shot theorem",
        );
    }
    // No pipeline pre-mark. A P > 0 buffer starts pre-filled; proving its
    // peak stays within C is the pipelined steady-state argument, deferred.
    if pipeline_p != 0 {
        return Err(
            "a buffer seq carries a pipeline pre-mark (P > 0); the pipelined steady-state \
             occupancy argument is not covered by the single-shot theorem",
        );
    }
    // Conservative source-layout guard: the Push must precede the Wait in
    // source order. (The buffer data-dependency forces this at firing
    // time regardless, but a Wait-before-Push *layout* is rejected to
    // keep the theorem hypotheses literal.)
    if f.wait_before_push {
        return Err(
            "a buffer seq has a Wait textually before its Push in source order (outside the \
             single-shot theorem's Push-before-Wait hypothesis)",
        );
    }
    // Capacity must be at least 1. `acfg_to_net` asserts `buffer >= 1`
    // upstream, but the theorem's bound (peak 1 <= C) needs C >= 1
    // literally, so we check it here rather than trust the invariant.
    match f.capacity {
        Some(c) if c >= 1 => Ok(()),
        _ => Err(
            "a buffer seq has capacity < 1 (upstream invariant violated); refusing the symbolic \
             bound (note: net expansion asserts on a zero-capacity place, so this surfaces \
             loudly either way)",
        ),
    }
}

/// Decide soundness of a COMMUNICATING net symbolically from the rolled
/// `acfg`, WITHOUT expanding it over the iteration space.
///
/// Returns [`SymbolicSoundness::ProvenSound`] iff EVERY buffer seq in the
/// ACFG satisfies the single-shot theorem (one Push + one Wait, both at
/// loop-depth 0, Push before Wait, no pipeline pre-mark, capacity `>= 1`)
/// — in which case the expanded net is bounded, deadlock-free and
/// conflict-free by the module theorem. Otherwise returns
/// [`SymbolicSoundness::NeedsExpansion`] with a reason naming the first
/// seq shape outside the subclass, and the caller falls back to the
/// expanded gate.
///
/// Precondition: the ACFG carries at least one `Xfer` (it is the
/// buffered/communicating case). A buffer-free ACFG is handled by the
/// faster structural check in [`super::net_soundness_symbolic`] and never
/// reaches here; if it does, this returns `ProvenSound` vacuously (no
/// seq to violate the theorem — which is correct, the buffer-free theorem
/// gives the same verdict).
///
/// Cost: `O(ACFG nodes)` — one walk collecting per-seq aggregates plus an
/// `O(#seqs)` classification. Independent of iteration counts.
pub fn analyze_communicating_net_symbolic(acfg: &ACFG) -> SymbolicSoundness {
    let mut facts: BTreeMap<SeqTag, SeqFacts> = BTreeMap::new();
    collect_seq_facts(&acfg.root, 0, &mut facts);

    for (seq, f) in &facts {
        let pipeline_p = acfg
            .pipeline_depth_for_seq
            .get(seq)
            .map(|d| d.get())
            .unwrap_or(0);
        if let Err(reason) = classify_seq(f, pipeline_p) {
            return SymbolicSoundness::NeedsExpansion(reason);
        }
    }

    SymbolicSoundness::ProvenSound
}
