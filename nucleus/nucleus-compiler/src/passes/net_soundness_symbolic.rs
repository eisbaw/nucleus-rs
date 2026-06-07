//! Symbolic (no-expansion) Petri-net soundness analysis
//! (TASK-0453.04, P4 of the rigour epic; PRD §8.1 / §8.4).
//!
//! ## Why this exists
//!
//! The production soundness gate ([`super::net_soundness::check_net_sound`])
//! runs on the net built by
//! [`acfg_to_net`](crate::passes::acfg_to_petri::acfg_to_net), which
//! **unrolls every `Repeat` over its full iteration range** (see that
//! module's "Why unroll" note). The resulting net carries roughly one
//! place and one transition per kernel firing, so both its construction
//! AND the gate's single-order replay are *linear in the number of
//! firings*. Measured on `07-matmul` under the naive (single-worker)
//! schedule: the $N\times N\times N$ triple loop produces about
//! $2N^3$ net nodes (8 199 at $N{=}16$, 524 295 at $N{=}64$) and the
//! gate's replay time tracks it linearly. That is cheap for the example
//! corpus (a few thousand firings) but would not scale to a
//! production-scale problem with billions of firings — the limitation
//! recorded in the thesis (`sec:res-quant-net` / `sec:fw-quant`).
//!
//! This pass lifts that bound *for a decidable subclass* by deciding
//! soundness directly from the **rolled** ACFG — whose size is a
//! function of the program text and worker count, NOT of the iteration
//! counts — without ever materialising the expanded net. For a program
//! in the subclass the decision is `O(ACFG nodes)`, independent of how
//! many times its loops run.
//!
//! ## The subclass: buffer-free nets, and why they are unconditionally sound
//!
//! [`acfg_to_net`](crate::passes::acfg_to_petri::acfg_to_net) emits
//! exactly two kinds of place:
//!
//! 1. **Per-worker control places** — `ctl_<worker>_<k>`. Each is
//!    capacity 1, produced by exactly one transition and consumed by
//!    exactly one (the next transition threaded through that worker, see
//!    `acfg_to_petri::NetBuilder::thread_through_worker`). The first
//!    control place of each worker carries `initial_marking = 1`; all
//!    others start empty.
//! 2. **Buffer places** — `buf_<data>_seq<n>`. Created **only** by an
//!    [`Xfer`](crate::acfg::ACFGNode::Xfer) placeholder
//!    (`acfg_to_petri::NetBuilder::buffer_place_for`). A `Push` deposits
//!    a token (`TtoP`), the matching `Wait` consumes one (`PtoT`), and
//!    one buffer place is shared across every unrolled iteration of an
//!    enclosing loop.
//!
//! Call a net **buffer-free** when it contains no buffer place —
//! equivalently, when its ACFG contains no `Xfer` node. We claim:
//!
//! > **Theorem.** A buffer-free net produced by `acfg_to_net` is always
//! > bounded, deadlock-free, and conflict-free; i.e.
//! > `check_net_sound` returns `Ok(())` on every such net.
//!
//! The proof is by elimination over the three rejection modes the gate
//! checks, each of which structurally **requires** a buffer place:
//!
//! - **Capacity overflow (boundedness).** Every control place has
//!   capacity 1, a single producer transition (which fires at most
//!   once), and starts with at most one token, so it never holds more
//!   than one — within capacity. The only way a place accumulates above
//!   its capacity is repeated deposits outracing consumption, which
//!   only a buffer place (many `Push`es into one shared place) can
//!   exhibit. No buffer place ⇒ no overflow.
//! - **Stall (deadlock).** `acfg_to_net` threads each transition onto
//!   the *most recently emitted* transition of each of its workers, so
//!   transition ids increase along every worker's control chain, and
//!   firing in source (id) order discharges every control dependency:
//!   when a transition is reached, each of its input control places was
//!   filled by an earlier-fired transition (or is an initial marking).
//!   Source order is therefore a legal, stall-free firing order, and
//!   `derive_firing_order` degenerates to it on a net with no nonzero
//!   buffer initial markings (see `boundedness::derive_firing_order`'s
//!   own docstring). The only transition that can stall is a `Wait`
//!   whose buffer place is empty — which again requires a buffer place.
//!   No buffer place ⇒ no stall.
//! - **Free-choice conflict (PRD §8.4(a)).** A conflict needs a place
//!   with two distinct consumer transitions co-enabled. Every control
//!   place has exactly one consumer, so the only place that can have
//!   `>= 2` consumers is a buffer place (many `Wait`s on one shared
//!   place — the benign serialised fan-out). No buffer place ⇒ no
//!   contested place ⇒ conflict-free by the fast path.
//!
//! Each rejection mode is thus impossible without a buffer place, so a
//! buffer-free net passes all three checks. ∎
//!
//! ## Soundness-equivalence (the load-bearing safety property)
//!
//! This pass is a *fast path*, never a replacement: a caller uses
//! [`SymbolicSoundness::ProvenSound`] to **skip** the expanded gate and
//! falls back to [`super::net_soundness::check_net_sound`] on
//! [`SymbolicSoundness::NeedsExpansion`]. The cardinal requirement
//! (P4 guardrail) is that this must never trade soundness for scaling:
//! it must reject every unsound net the expanded gate rejects.
//!
//! It does, by construction. We only ever return `ProvenSound` for a
//! buffer-free ACFG, and the Theorem says the expanded gate returns
//! `Ok(())` on every buffer-free net — so skipping the expanded gate on
//! that path skips only a guaranteed-`Ok` check. Every net the expanded
//! gate *could* reject (overflow / stall / conflict) has a buffer place,
//! hence an `Xfer`, hence is classified [`SymbolicSoundness::NeedsExpansion`]
//! and routed to the unchanged expanded gate. The fast path is therefore
//! sound-equivalent-or-stronger: it changes *when* the expanded analysis
//! runs, never *whether* an unsound net is caught.
//!
//! ## Honest scope (the residual)
//!
//! The decidable subclass is exactly the **buffer-free** nets:
//! single-worker programs (e.g. every naive schedule, including the
//! `07-matmul` triple loop the limitation cites) and any multi-worker
//! program with no cross-worker transfers. A program that *does*
//! distribute work and communicate — the loop-carried `Push`/`Wait`
//! shape of the distributed schedules — still falls back to the
//! expanded, linear-in-firings gate. Lifting the bound there needs a
//! periodicity / steady-state argument over the shared buffer place's
//! occupancy under the greedy firing order, which is left as future
//! work; doing it imprecisely would risk missing a real overflow, which
//! the guardrail forbids. So this pass lifts the scaling bound for the
//! no-communication subclass and documents the buffered/distributed
//! case as the honest residual.

use crate::acfg::{ACFGNode, ACFG};

/// Outcome of [`analyze_net_soundness_symbolic`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolicSoundness {
    /// The net is proven sound directly from the rolled ACFG, without
    /// expanding it over the iteration space. For this subclass this is
    /// equivalent to `check_net_sound(acfg_to_net(acfg)) == Ok(())`
    /// (see the module Theorem). The caller may skip the expanded gate.
    ProvenSound,
    /// The ACFG is outside the symbolic analysis's decidable subclass
    /// (it carries cross-worker transfers, hence buffer places whose
    /// boundedness/liveness depends on the firing-order interleaving).
    /// The caller MUST fall back to the expanded
    /// [`super::net_soundness::check_net_sound`]. The `&'static str` is
    /// a human-readable reason for `NUC_TRACE` diagnostics.
    NeedsExpansion(&'static str),
}

/// Decide net soundness symbolically from the **rolled** ACFG, without
/// expanding the net over the iteration space.
///
/// Returns [`SymbolicSoundness::ProvenSound`] iff the ACFG is
/// buffer-free (no [`Xfer`](crate::acfg::ACFGNode::Xfer) node), which by
/// the module Theorem implies the expanded net is bounded, deadlock-free
/// and conflict-free. Otherwise returns
/// [`SymbolicSoundness::NeedsExpansion`] and the caller falls back to the
/// expanded gate.
///
/// Cost: `O(ACFG nodes)` — a single walk of the rolled tree. It does NOT
/// descend a `Repeat` body per iteration; it inspects each loop body once
/// regardless of the range, so the cost is independent of the iteration
/// counts (the whole point — contrast `acfg_to_net`, which unrolls).
pub fn analyze_net_soundness_symbolic(acfg: &ACFG) -> SymbolicSoundness {
    if contains_xfer(&acfg.root) {
        return SymbolicSoundness::NeedsExpansion(
            "net carries cross-worker transfers (buffer places); boundedness/liveness of a \
             shared buffer place depends on the firing-order interleaving, which is outside \
             the buffer-free symbolic subclass — falling back to the expanded gate",
        );
    }
    SymbolicSoundness::ProvenSound
}

/// Does `node` (or any descendant) contain an
/// [`Xfer`](crate::acfg::ACFGNode::Xfer) placeholder?
///
/// A `Repeat` body is inspected **once**, not per iteration: the
/// presence of an `Xfer` inside a loop body does not depend on how many
/// times the loop runs, so one structural descent suffices. This is what
/// keeps the analysis independent of the iteration counts.
fn contains_xfer(node: &ACFGNode) -> bool {
    match node {
        ACFGNode::Xfer(_) => true,
        ACFGNode::Operation(_) | ACFGNode::Sync(_) => false,
        ACFGNode::Sequence(children) => children.iter().any(contains_xfer),
        ACFGNode::Repeat { body, .. } => contains_xfer(body),
    }
}
