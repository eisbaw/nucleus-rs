//! Combined Petri-net soundness gate (TASK-0368, PRD §8.1 / §8.4).
//!
//! This module bundles the analysis passes
//! ([`boundedness`](crate::passes::boundedness),
//! [`deadlock`](crate::passes::deadlock), and the conflict-free
//! [`check_conflict_free`]) into a single entry point,
//! [`check_net_sound`], that the production driver runs on EVERY build
//! (TASK-0368). Before TASK-0368 the analyses ran only in the test
//! suite and under the `--emit-pn` inspection branch; the shipping
//! compiler enforced net soundness only structurally (via TtoP-arc
//! elision and the ad-hoc ACFG guards in the inject passes). Wiring
//! the analyses in as a hard gate is what makes PRD §8's claim
//! ("analyses fall out as standard properties; failures are compile
//! errors") literally true of the shipping compiler, not just the
//! test suite.
//!
//! The conflict-free pass (TASK-0421) closes the last §8.4 enforcement
//! gap: §8.4(b) bounded and §8.4(c) acyclic already had per-build gates
//! (`check_bounded` / `check_deadlock_free`), §8.4(d) no-colour is a
//! type-level guarantee, but §8.4(a) no-free-choice/no-conflict — the
//! precondition the single-order replay rests on — had ZERO asserting
//! code until [`check_conflict_free`] was added here.
//!
//! ## Method (and its honest scope)
//!
//! The gate is **exact-replay over one deterministic firing order**,
//! not a general reachability/coverability engine:
//!
//! 1. [`check_conflict_free`] replays the order and rejects if any
//!    reachable marking has a place with `>= 2` consumer transitions
//!    simultaneously enabled — i.e. the net violates PRD §8.4(a)'s
//!    no-free-choice / statically-determined-order precondition. This
//!    runs FIRST because steps 2-3's single-order replay is only sound
//!    *given* this precondition (see [`check_net_sound`]'s body).
//! 2. [`derive_firing_order`] linearises the net into a single
//!    deterministic firing sequence (source order + marking-aware
//!    reordering; see its docstring).
//! 3. [`check_bounded`] replays that order and rejects if any firing
//!    would push a place above its declared capacity.
//! 4. [`check_deadlock_free`] replays that order and rejects if any
//!    step stalls (an input place lacks tokens).
//!
//! This is sound for v2's restricted nets specifically because their
//! firing order is *statically determined* (PRD §8.4: no free-choice,
//! no confusion, no conflicts — partially checked by step 1, an
//! under-approximating tripwire over the derived order, NOT a complete
//! coverability proof; see [`check_conflict_free`]'s "Honest
//! limitation") and they are *bounded by construction*.
//! For that subclass the single-order replay is exactly equivalent to
//! reachability (see the soundness justification in
//! [`deadlock`](crate::passes::deadlock)'s module doc). A future
//! relaxation that admitted true run-time nondeterminism would need a
//! coverability check, NOT this replay — do not read the gate as a
//! general Petri-net model checker.
//!
//! ## Ordering, and semantically-correct error labelling
//!
//! Both passes replay the *same* single firing order, so a single
//! replay can hit either of two distinct `FireError`s: a capacity
//! overflow or a stall (an input place lacking tokens). Each pass
//! treats the *other's* failure mode as an "undefined" sentinel rather
//! than silently returning `Ok`:
//!
//! - `check_bounded` returns `BoundednessError::InvalidFiringOrder` on
//!   a stall — which its own docstring calls "a deadlock-territory
//!   problem, not a boundedness one".
//! - `check_deadlock_free` returns `DeadlockError::CapacityExceeded` on
//!   an overflow — "a boundedness violation, not a deadlock".
//!
//! Labelling a stall as a `Boundedness` error (or an overflow as a
//! `Deadlock` error) would be a misleading compile message. So the
//! gate does NOT blindly forward the first pass's error. It runs
//! `check_bounded` first and:
//!
//! - maps a genuine `CapacityExceeded` (or `UnknownTransition`) to
//!   [`PetriAnalysisError::Boundedness`];
//! - on `InvalidFiringOrder` (a stall, not an overflow) it falls
//!   through to `check_deadlock_free`, which diagnoses the same stall
//!   precisely as `DeadlockError::Stalled` and is mapped to
//!   [`PetriAnalysisError::Deadlock`].
//!
//! The net effect: an overflowing net is reported as a boundedness
//! failure and a deadlocking net as a deadlock failure, regardless of
//! which underlying pass physically observed the `FireError` first.
//! Boundedness is checked first because the capacity question is
//! logically prior to the liveness question (PRD §8.2's
//! buffer-sufficiency framing).
//!
//! ## Why a library function rather than inline driver code
//!
//! The driver (`nucleus/driver/src/main.rs`) calls this one function
//! and stringifies the typed error into its `Err(String)` channel.
//! Keeping the gate as a tested `pub fn` here is what lets the
//! negative regression test (`tests/net_soundness.rs`) exercise the
//! *exact* gate the driver runs, against hand-built unsound nets — we
//! cannot drive an unsound net through the full driver because no
//! valid schedule produces one (the structural inject-pass guards
//! prevent it). The gate is therefore a *provably-dead-today tripwire*
//! on every shipping schedule; the negative test pins it at the
//! function level so a future regression in the inject passes (one
//! that DID emit an unsound net) would be caught at build time rather
//! than shipping as a runtime hang or buffer overrun.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::petri::{ArcKind, Marking, Net, PlaceId, TransitionId};

use super::boundedness::{check_bounded, derive_firing_order, BoundednessError};
use super::deadlock::{check_deadlock_free, DeadlockError};

// --------------------------------------------------------------------
// PetriAnalysisError
// --------------------------------------------------------------------

/// Why [`check_net_sound`] rejected a net.
///
/// One variant per underlying analysis. The variant preserves the full
/// typed payload of the underlying error so callers (and tests) can
/// inspect the offending place/transition without re-running the pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PetriAnalysisError {
    /// The net failed the boundedness check: some firing in the
    /// derived order would exceed a place's declared capacity (or the
    /// derived order is not a legal interleaving — see
    /// [`BoundednessError`]).
    Boundedness(BoundednessError),
    /// The net failed the deadlock check: the derived order stalls
    /// because an input place lacks tokens (see [`DeadlockError`]).
    Deadlock(DeadlockError),
    /// The net failed the conflict-free / no-free-choice check
    /// (PRD §8.4(a)): at some reachable marking along the derived
    /// firing order, two or more distinct transitions that consume from
    /// the *same* place are simultaneously enabled, so which one fires
    /// is decided by token availability at run time, not statically.
    ///
    /// This violates the precondition the whole single-order replay
    /// rests on (see the module doc). The payload names the contested
    /// place and the `>= 2` consumer transitions that were co-enabled.
    /// See [`check_conflict_free`].
    ConflictingChoice(ConflictError),
}

impl std::fmt::Display for PetriAnalysisError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Delegate to the underlying error's Display; each already
        // prefixes itself ("boundedness: ..." / "deadlock: ...") so the
        // failure mode is unambiguous in the driver's stringified
        // message.
        match self {
            PetriAnalysisError::Boundedness(e) => write!(f, "{e}"),
            PetriAnalysisError::Deadlock(e) => write!(f, "{e}"),
            PetriAnalysisError::ConflictingChoice(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for PetriAnalysisError {}

// --------------------------------------------------------------------
// ConflictError
// --------------------------------------------------------------------

/// Why a [`check_conflict_free`] call rejected the net.
///
/// Carries the contested place and the conflicting consumer
/// transitions (with echoed names) so the driver can build a
/// user-facing message without re-indexing into the net.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictError {
    /// At a reachable marking, two or more distinct transitions that
    /// consume from `place` were each enabled. The choice of which fires
    /// is therefore decided by run-time token availability, not by the
    /// static order — a free-choice conflict (PRD §8.4(a)).
    FreeChoice {
        /// The contested input place.
        place: PlaceId,
        /// Echoed place name for diagnostics.
        place_name: String,
        /// The co-enabled consumer transitions (`>= 2`), sorted by
        /// [`TransitionId`] for a stable message. Each entry is the
        /// id plus its echoed name.
        transitions: Vec<(TransitionId, String)>,
        /// Minimum firings from the initial marking to a marking
        /// exhibiting the conflict (the BFS depth at which the
        /// coverability search first reaches a conflicting marking).
        /// Zero-based; a conflict at the initial marking reports `0`.
        /// Useful for pointing the user at a source position once the
        /// order -> span map exists.
        position: usize,
    },
}

impl std::fmt::Display for ConflictError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConflictError::FreeChoice {
                place_name,
                transitions,
                position,
                ..
            } => {
                let names: Vec<&str> =
                    transitions.iter().map(|(_, n)| n.as_str()).collect();
                write!(
                    f,
                    "conflict: place '{}' is a free-choice conflict {} firing(s) from the initial \
                     marking: transitions [{}] are simultaneously enabled and all consume from it, \
                     so which fires is not statically determined (PRD §8.4(a))",
                    place_name,
                    position,
                    names.join(", ")
                )
            }
        }
    }
}

impl std::error::Error for ConflictError {}

// --------------------------------------------------------------------
// check_conflict_free
// --------------------------------------------------------------------

/// Verify the net has PRD §8.4(a)'s no-free-choice / conflict-free
/// shape: no place ever has two distinct consumer transitions
/// simultaneously enabled at a reachable marking.
///
/// ## What "conflict-free" means here, and why it is order-aware
///
/// PRD §8.4(a) requires a *statically determined* firing order — "order
/// decided at compile time, not by token availability at run time". The
/// Petri-net realisation of "token availability decides" is a
/// **free-choice conflict**: a place `p` whose token can enable either
/// of two transitions `t1`, `t2`, so the net must *choose* which one
/// consumes it. If only one of `t1`/`t2` is enabled at every reachable
/// marking, there is no choice — the order is forced by structure, not
/// by run-time tokens.
///
/// A purely *structural* predicate ("`p` has `PtoT` arcs to `>= 2`
/// distinct transitions ⇒ conflict") is WRONG for v2 nets and would
/// false-reject every shipping schedule with a transfer inside a loop.
/// `acfg_to_net` **unrolls** `Repeat` bodies (TASK-0026): a transfer
/// with a fixed `SeqTag` inside an N-iteration loop emits one buffer
/// place consumed by N distinct unrolled `Wait` transitions. Those N
/// waits are NOT in conflict — each is gated behind its own per-worker
/// control-place predecessor, so at any reachable marking at most ONE
/// of them is enabled. (This was confirmed empirically over the full
/// e2e corpus; see TASK-0421 AC#1: zero shipping nets have any place
/// with two co-enabled consumers, but several have a buffer place with
/// `PtoT` arcs to many distinct waits — exactly the benign serialised
/// fan-out a structural predicate would mis-flag.)
///
/// So the predicate is **reachability-aware**: for a small contested net we
/// run a deterministic bounded coverability BFS over the net's reachable,
/// capacity-respecting markings and, at EACH reachable marking, check that
/// no place has `>= 2` of its consumer transitions enabled at once. Two
/// guards keep this affordable (see "## Cost"): a fast path short-circuits
/// the BFS entirely when no place even has `>= 2` consumers, and a
/// transition-count guard routes LARGE contested nets (real multi-worker
/// shipping nets DO have contested places — the benign serialised
/// cross-worker `Wait` fan-out) to the historical single-order replay
/// instead of an intractable product-state-space BFS.
///
/// ## "Enabled" here = consumption-enabled (capacity ignored)
///
/// A transition is *consumption-enabled* at a marking iff every place
/// it consumes from holds at least its required weight. We deliberately
/// do NOT fold in the output-capacity check that
/// [`crate::petri::Net::enabled_transitions`] applies: capacity is the
/// boundedness gate's concern, and a place that enables two consumers
/// is a conflict regardless of whether one of them would later
/// overflow. Folding capacity in could MASK a real conflict (one
/// consumer happens to be capacity-blocked at that instant), which is
/// the unsafe direction for a soundness tripwire.
///
/// ## Disposition (provably-dead-today tripwire)
///
/// Like [`check_bounded`]/[`check_deadlock_free`], this is a
/// conservative tripwire: the `acfg_to_net` control-place threading
/// makes a free-choice conflict structurally impossible on every
/// shipping schedule today (each transition consumes a fresh
/// single-marked control place, serialising every worker). The check
/// exists to catch a FUTURE inject-pass regression that emitted a net
/// where a place co-enables two consumers at ANY reachable marking —
/// which the boundedness/deadlock gates would NOT catch (they replay a
/// single order and would silently pass on it). The bite tests in
/// `tests/net_soundness.rs` pin the reject path at the function level
/// (one conflict visible at the initial marking, one reachable only on
/// an off-order interleaving the old single-order replay missed).
///
/// ## Completeness — and the two residuals (TASK-0427)
///
/// For SMALL contested nets (`<= CONFLICT_BFS_TRANSITION_LIMIT`
/// transitions) whose reachable state space also fits within
/// [`CONFLICT_STATE_SPACE_CAP`] markings, this gate is COMPLETE: the
/// coverability BFS visits EVERY reachable, capacity-respecting marking
/// and flags a conflict that co-enables two consumers at ANY of them,
/// not only the markings one derived order happens to visit. This is a
/// strict improvement over the prior single-order replay, which inspected
/// only the markings along one [`derive_firing_order`] and therefore
/// missed a conflict reachable solely on a *different* interleaving (two
/// producers both depositing into a place before either consumer fires —
/// the architect's counterexample in TASK-0421, pinned by
/// `off_order_free_choice_conflict_now_detected` in
/// `tests/net_soundness.rs`).
///
/// There are TWO residuals where the gate REVERTS to the historical
/// single-order replay (an under-approximation: false negatives possible,
/// NEVER false positives), each emitting a `NUC_TRACE` advisory:
///
/// 1. **Large contested net** (`> CONFLICT_BFS_TRANSITION_LIMIT`
///    transitions). The BFS explores the net's reachable PRODUCT state
///    space, which on a multi-worker net is combinatorial; running it on,
///    e.g., the 522-transition 16-jacobi/distributed × mp-tcp-event gate
///    net (24 contested places — the benign serialised cross-worker `Wait`
///    fan-out) churns for ~15 minutes. So a net above the limit is NOT run
///    through the BFS at all (see [`CONFLICT_BFS_TRANSITION_LIMIT`]). This
///    is the COMMON regime for real shipping multi-worker nets — contrary
///    to the original TASK-0421 premise that "no shipping net has a
///    contested place", several do (the same benign serialised fan-out the
///    `shipping_shaped_unrolled_loop_buffer_passes_no_false_reject` pin
///    accepts, but at multi-worker scale).
/// 2. **Small net that nonetheless exceeds the marking cap.** A net within
///    the transition limit but whose reachable state space still exceeds
///    [`CONFLICT_STATE_SPACE_CAP`]: the BFS truncates and falls back. A
///    defensive bound only; no net within the transition limit is known to
///    reach it.
///
/// So the gate is "complete for small contested nets within both caps;
/// single-order under-approximation for large/blowup nets". In ALL regimes
/// the gate never false-positives: it never rejects a valid build (the
/// single-order replay is sound; confirmed by the e2e baseline holding and
/// the `shipping_shaped_unrolled_loop_buffer_passes_no_false_reject`
/// no-false-reject pin). v2's structural inject-pass guards make a genuine
/// free-choice conflict impossible on every shipping schedule regardless,
/// so the residual under-approximation has no effect on any real net; the
/// completeness upgrade buys teeth against a FUTURE small-net inject-pass
/// regression.
///
/// ## Determinism
///
/// The BFS is deterministic: the frontier is a FIFO queue, successors are
/// generated by firing transitions in [`TransitionId`] order, the visited
/// set dedups on a canonical marking key, and the per-marking enabled-set
/// scan iterates the contested places in [`PlaceId`] order. So the
/// reported conflict — the first contested place (in id order) at the
/// minimum-depth conflicting marking the FIFO reaches — and its reported
/// `position` (BFS depth) are byte-stable across runs. The cap-fallback
/// path inherits `derive_firing_order`'s determinism.
///
/// ## Cost (the perf-pin contract, TASK-0377 / TASK-0421 / TASK-0427)
///
/// The common case — *no* place has `>= 2` consumers — is detected in
/// O(A) (one arc pass) and returns immediately, with NO BFS. This is the
/// `gate_stays_near_linear_under_large_net` perf-pin net (16000
/// transitions, each place with exactly ONE consumer ⇒ no contested place
/// ⇒ fast path) and many shipping nets. The perf-pin claim holds: the BFS
/// never runs on it.
///
/// When a place HAS `>= 2` consumers, the next gate is the **transition
/// count** ([`CONFLICT_BFS_TRANSITION_LIMIT`]): a LARGE contested net
/// (real multi-worker shipping nets — e.g. 16-jacobi/distributed has 522
/// transitions, 24 contested places) does NOT run the BFS; it does ONE
/// `derive_firing_order` + a single-order replay (O(A) + O(order)), the
/// historical cost. This is what keeps the e2e per-cell build fast and
/// bit-identical — running the BFS on those nets would churn for minutes.
///
/// Only a SMALL contested net (`<= CONFLICT_BFS_TRANSITION_LIMIT`
/// transitions — the architect's counterexample, the synthetic tests)
/// runs the coverability BFS: O(reachable-markings × T) bounded by
/// [`CONFLICT_STATE_SPACE_CAP`], reusing one `sim` and firing in place
/// (O(deg(t)) per successor, no per-step net clone), with each marking's
/// conflict check touching only the contested places' consumers via a
/// per-transition `needs` table precomputed in O(A). Such nets are tiny,
/// so correctness/determinism/termination — not throughput — are the
/// requirements; the transition limit and marking cap guarantee
/// termination. The `gate_stays_near_linear_under_large_net` pin in
/// `tests/boundedness.rs` guards the fast-path near-linear bound.
pub fn check_conflict_free(net: &Net) -> Result<(), ConflictError> {
    check_conflict_free_with_order(net, None)
}

/// Internal worker. For a small contested net the conflict check is a
/// coverability BFS over reachable markings that does NOT use `order`.
/// `order` is used by the single-order fallback
/// ([`conflict_free_single_order_fallback`]), which runs when the net is
/// LARGER than [`CONFLICT_BFS_TRANSITION_LIMIT`] (the common case for real
/// multi-worker shipping nets — their BFS product state space is
/// intractable) or when a small net's BFS exceeds the marking cap. The
/// fallback reuses the firing order [`check_net_sound`] already derived
/// (the passes share one order), avoiding a redundant `derive_firing_order`
/// call; `None` derives it on demand (the `pub fn check_conflict_free`
/// entry point + tests pass `None`).
fn check_conflict_free_with_order(
    net: &Net,
    order: Option<&[TransitionId]>,
) -> Result<(), ConflictError> {
    // Per place, the set of distinct transitions that consume from it
    // (have at least one PtoT arc place->transition). BTreeMap/BTreeSet
    // for deterministic iteration. O(A).
    let mut consumers: BTreeMap<PlaceId, std::collections::BTreeSet<TransitionId>> =
        BTreeMap::new();
    for a in &net.arcs {
        if a.kind == ArcKind::PtoT {
            consumers.entry(a.place).or_default().insert(a.transition);
        }
    }
    // Restrict to *contested* places — those with >= 2 distinct
    // consumers. A place with < 2 consumers can never be a free-choice
    // conflict at any marking, so it is irrelevant to the scan.
    consumers.retain(|_, cset| cset.len() >= 2);

    // Fast path: no contested place ⇒ structurally conflict-free, no
    // replay needed. This covers MANY shipping nets and the perf-pin net
    // (16000 transitions, one consumer per place), so the gate stays O(A)
    // there. NOTE: contrary to the original TASK-0421 premise, NOT every
    // shipping net hits this fast path — real multi-worker nets (e.g.
    // 16-jacobi/distributed) have contested places (benign serialised
    // cross-worker `Wait` fan-out) and fall through to the size guard
    // below.
    if consumers.is_empty() {
        return Ok(());
    }

    // Precompute each (relevant) transition's input needs once, O(A),
    // so the per-marking enabled check is O(distinct input places of t)
    // rather than an all-arcs scan. We only need needs for transitions
    // that consume from a contested place.
    let relevant: std::collections::BTreeSet<TransitionId> =
        consumers.values().flatten().copied().collect();
    let mut needs: BTreeMap<TransitionId, BTreeMap<PlaceId, u32>> = BTreeMap::new();
    for a in &net.arcs {
        if a.kind == ArcKind::PtoT && relevant.contains(&a.transition) {
            *needs
                .entry(a.transition)
                .or_default()
                .entry(a.place)
                .or_insert(0) += a.weight;
        }
    }

    let index = net.build_arc_index();

    // SIZE GUARD (TASK-0427, empirically load-bearing). A contested place
    // exists. The coverability BFS below explores the net's reachable
    // PRODUCT state space, which on a multi-worker net is combinatorial in
    // the number of transitions/control-chains. Contrary to the original
    // TASK-0421 premise, real shipping nets DO have contested places: the
    // 16-jacobi/distributed × mp-tcp-event gate net has 1179 places / 522
    // transitions / 24 contested places (the benign serialised cross-worker
    // Wait fan-out — the same shape `shipping_shaped_unrolled_loop_buffer_
    // passes_no_false_reject` pins, but at multi-worker scale). On such a
    // net the BFS would churn for ~15 minutes toward the visited cap before
    // falling back — blowing the e2e per-cell budget. So we run the BFS
    // ONLY on nets small enough for the state space to be tractable; larger
    // nets use the historical single-order replay directly. This keeps the
    // completeness upgrade honest (complete for SMALL contested nets — the
    // architect's counterexample and the synthetic tests; single-order
    // under-approximation for large ones, exactly as before TASK-0427) AND
    // preserves the e2e bit-identity + perf invariants. The single-order
    // replay never false-rejects, so the large-net path stays SOUND.
    if net.transitions.len() > CONFLICT_BFS_TRANSITION_LIMIT {
        crate::nuc_trace!(
            "check_conflict_free: net has {} transitions (> the {}-transition BFS limit) \
             and {} contested place(s); skipping the coverability BFS (its product state \
             space is intractable at this size) and using the single derived-order replay, \
             so the result is the single-order UNDER-APPROXIMATION for this net (sound — \
             never false-rejects — but a conflict reachable only on an off-order \
             interleaving may be missed). v2's structural inject-pass guards make any such \
             conflict impossible on shipping schedules; this path is a defensive residual.",
            net.transitions.len(),
            CONFLICT_BFS_TRANSITION_LIMIT,
            consumers.len()
        );
        return conflict_free_single_order_fallback(net, order, &consumers, &needs, &index);
    }

    // Small contested net ⇒ run the coverability BFS over ALL reachable,
    // capacity-respecting markings (not just one order's markings).
    let mut sim = net.clone();
    sim.reset_to_initial();

    // Frontier of (marking, depth). `visited` dedups by the canonical key
    // (sorted (place.0, count) tuples). `Marking::set(p, 0)` removes the
    // entry, so the BTreeMap never holds a zero count — two markings that
    // are equal under `Marking::get` therefore map to the SAME key, which
    // is what makes the dedup correct (no absent-vs-zero split).
    let mut frontier: VecDeque<(Marking, usize)> = VecDeque::new();
    let mut visited: BTreeSet<Vec<(u32, u32)>> = BTreeSet::new();
    frontier.push_back((sim.current_marking.clone(), 0));
    visited.insert(marking_key(&sim.current_marking));

    while let Some((marking, depth)) = frontier.pop_front() {
        // Check the conflict predicate at THIS reachable marking. The
        // predicate is byte-identical to the old single-order path; only
        // the SET of markings inspected widened. `position` is the BFS
        // depth: the minimum firings to reach a conflicting marking.
        if let Some(conflict) =
            find_conflict_at_marking(net, &marking, &consumers, &needs, depth)
        {
            return Err(conflict);
        }

        // Defensive cap: v2 nets are tiny, so this only fires on a
        // pathological future net. We do NOT hard-error (panic-on-valid-
        // input is the antipattern this gate rejects); instead we FALL
        // BACK to the bounded single-order replay (the historical
        // behaviour) and emit an advisory that the result is the
        // single-order UNDER-APPROXIMATION for this net (honest residual).
        if visited.len() >= CONFLICT_STATE_SPACE_CAP {
            crate::nuc_trace!(
                "check_conflict_free: coverability BFS truncated at the {}-marking cap \
                 for this net; falling back to the single derived-order replay, so the \
                 result is the single-order UNDER-APPROXIMATION (a conflict reachable only \
                 on an unvisited off-order interleaving above the cap may be missed)",
                CONFLICT_STATE_SPACE_CAP
            );
            return conflict_free_single_order_fallback(net, order, &consumers, &needs, &index);
        }

        // Generate successors deterministically: fire each transition in
        // TransitionId order from this marking. A transition that cannot
        // fire (token-stall or capacity) is simply not enqueued — this
        // restricts the BFS to reachable, in-bounds markings (capacity-
        // respecting reachability; the conflict PREDICATE still ignores
        // capacity, which is correct and unchanged).
        //
        // We reuse the single `sim` rather than cloning the whole net per
        // transition: before each attempt we restore `sim.current_marking`
        // to this frontier marking, then `fire_in_place` mutates only the
        // places incident to the fired transition (O(deg(t))). On failure
        // `fire_in_place` leaves the marking untouched, so the restore at
        // the top of the next iteration is what guarantees independence.
        for ti in 0..net.transitions.len() {
            let tid = TransitionId(ti as u32);
            sim.current_marking = marking.clone();
            if sim.fire_in_place(tid, &index).is_err() {
                continue;
            }
            let key = marking_key(&sim.current_marking);
            if visited.insert(key) {
                frontier.push_back((sim.current_marking.clone(), depth + 1));
            }
        }
    }

    Ok(())
}

/// Defensive upper bound on the number of distinct markings the
/// coverability BFS in [`check_conflict_free`] will visit before falling
/// back to the single-order under-approximation. v2 nets are tiny (a
/// handful of control places per worker), so this is never reached on a
/// real net; it exists purely to guarantee termination on a
/// pathological future net rather than spinning. Mirrors the
/// `STATE_SPACE_CAP` used by the `proptest_petri.rs` reachability oracle.
const CONFLICT_STATE_SPACE_CAP: usize = 50_000;

/// Maximum transition count for which [`check_conflict_free`] runs the
/// coverability BFS. Above this, a contested net uses the historical
/// single-order replay directly (no BFS).
///
/// Why a size guard at all (TASK-0427, empirically required): the BFS
/// explores the net's reachable PRODUCT state space, which on a
/// MULTI-WORKER net is combinatorial in the number of concurrent
/// control-chains/transitions. Real shipping nets DO contain contested
/// places (e.g. 16-jacobi/distributed × mp-tcp-event: 522 transitions, 24
/// contested places — the benign serialised cross-worker `Wait` fan-out),
/// so the fast path does NOT short-circuit them; without this guard the
/// BFS churns for ~15 minutes toward [`CONFLICT_STATE_SPACE_CAP`] before
/// the cap-fallback fires, blowing the e2e per-cell budget. The guard
/// keeps the BFS on nets small enough to be tractable (the architect's
/// 5-transition TASK-0421 counterexample, the synthetic test nets) and
/// routes larger nets to the sound single-order replay.
///
/// 64 is generous for any plausible hand-built genuine-conflict net while
/// excluding multi-worker shipping nets by an order of magnitude. Small
/// nets reaching this limit are single-worker-shaped (strictly
/// control-chain serialised ⇒ near-linear reachable markings), so the BFS
/// stays fast on every net it actually runs on; the
/// [`CONFLICT_STATE_SPACE_CAP`] visited-count cap is the secondary
/// defensive bound for any small-but-concurrent future net.
const CONFLICT_BFS_TRANSITION_LIMIT: usize = 64;

/// Canonical key for a [`Marking`] so it can live in a `BTreeSet` for the
/// BFS visited-set. `Marking` does not implement `Ord`; we extract the
/// sorted `(place.0, count)` tuples. Because `Marking::set(p, 0)` removes
/// the entry, the backing map never holds a zero count, so two markings
/// equal under `Marking::get` produce the SAME key (no absent-vs-zero
/// split) — the property the dedup relies on. Mirrors
/// `tests/proptest_petri.rs::marking_key`.
fn marking_key(m: &Marking) -> Vec<(u32, u32)> {
    let mut v: Vec<(u32, u32)> = m.0.iter().map(|(p, n)| (p.0, *n)).collect();
    v.sort();
    v
}

/// Cap-exceeded fallback: the historical single-derived-order replay.
/// Used ONLY when the coverability BFS hits [`CONFLICT_STATE_SPACE_CAP`]
/// without finding a conflict (a pathological future net). It inspects
/// only the markings along ONE order, so it is the under-approximation
/// documented in [`check_conflict_free`]'s "Honest limitation" section.
/// `order` is the shared firing order [`check_net_sound`] derived (reused
/// to avoid a redundant `derive_firing_order`); `None` derives on demand.
fn conflict_free_single_order_fallback(
    net: &Net,
    order: Option<&[TransitionId]>,
    consumers: &BTreeMap<PlaceId, BTreeSet<TransitionId>>,
    needs: &BTreeMap<TransitionId, BTreeMap<PlaceId, u32>>,
    index: &crate::petri::ArcIndex,
) -> Result<(), ConflictError> {
    let owned_order;
    let order: &[TransitionId] = match order {
        Some(o) => o,
        None => {
            owned_order = derive_firing_order(net);
            &owned_order
        }
    };

    let mut sim = net.clone();
    sim.reset_to_initial();

    // Inspect each marking along the order, then fire to the next.
    // `position` counts firings already committed (position == 0 is the
    // initial marking). This is the historical behaviour, preserved
    // verbatim as the above-cap residual.
    for position in 0..=order.len() {
        if let Some(conflict) =
            find_conflict_at_marking(net, &sim.current_marking, consumers, needs, position)
        {
            return Err(conflict);
        }
        if let Some(&tid) = order.get(position) {
            // A stall/overflow here is a boundedness/deadlock concern,
            // not a conflict; we simply stop the scan (the order cannot
            // advance).
            if sim.fire_in_place(tid, index).is_err() {
                break;
            }
        }
    }

    Ok(())
}

/// Look for a free-choice conflict at one marking: a contested place
/// with `>= 2` of its consumer transitions consumption-enabled. Returns
/// the first such place in [`PlaceId`] order (deterministic). `consumers`
/// holds ONLY contested places (`>= 2` consumers, pre-filtered); `needs`
/// is the precomputed input-need table keyed by transition.
fn find_conflict_at_marking(
    net: &Net,
    marking: &crate::petri::Marking,
    consumers: &BTreeMap<PlaceId, std::collections::BTreeSet<TransitionId>>,
    needs: &BTreeMap<TransitionId, BTreeMap<PlaceId, u32>>,
    position: usize,
) -> Option<ConflictError> {
    for (place, cset) in consumers {
        // Collect the consumers that are consumption-enabled at this
        // marking (every input place holds enough tokens). Capacity is
        // deliberately ignored (see the fn doc).
        let mut enabled: Vec<(TransitionId, String)> = Vec::new();
        for &tid in cset {
            if is_consumption_enabled(needs, tid, marking) {
                enabled.push((tid, name_of_transition(net, tid)));
            }
        }
        if enabled.len() >= 2 {
            // `cset` is a BTreeSet so `enabled` is already in
            // TransitionId order.
            return Some(ConflictError::FreeChoice {
                place: *place,
                place_name: name_of_place(net, *place),
                transitions: enabled,
                position,
            });
        }
    }
    None
}

/// Is `t` consumption-enabled at `marking`? I.e. does every place `t`
/// consumes from (summing parallel `PtoT` arc weights, precomputed in
/// `needs`) hold at least the required tokens? Output capacity is
/// intentionally NOT checked (see [`check_conflict_free`]'s "Enabled
/// here" note). A transition with no entry in `needs` consumes nothing
/// and is trivially enabled.
fn is_consumption_enabled(
    needs: &BTreeMap<TransitionId, BTreeMap<PlaceId, u32>>,
    t: TransitionId,
    marking: &crate::petri::Marking,
) -> bool {
    match needs.get(&t) {
        Some(places) => places.iter().all(|(p, n)| marking.get(*p) >= *n),
        None => true,
    }
}

fn name_of_place(net: &Net, p: PlaceId) -> String {
    net.places
        .get(p.0 as usize)
        .map(|pl| pl.name.clone())
        .unwrap_or_else(|| format!("<unknown place {:?}>", p))
}

fn name_of_transition(net: &Net, t: TransitionId) -> String {
    net.transitions
        .get(t.0 as usize)
        .map(|tr| tr.name.clone())
        .unwrap_or_else(|| format!("<unknown transition {:?}>", t))
}

// --------------------------------------------------------------------
// check_net_sound
// --------------------------------------------------------------------

/// Run the full Petri-net soundness gate on `net`.
///
/// Checks conflict-freedom (PRD §8.4(a)) first, then derives a
/// deterministic firing order and checks boundedness, then deadlock-
/// freedom against that order. Returns `Ok(())` iff the net passes all
/// three.
///
/// The error returned is labelled by the *semantic* failure mode, not
/// by which pass physically observed it (see the module doc's
/// "semantically-correct error labelling"): an overflow is a
/// [`PetriAnalysisError::Boundedness`] and a stall is a
/// [`PetriAnalysisError::Deadlock`], even though `check_bounded` (which
/// runs first) reports a stall as its own `InvalidFiringOrder`
/// sentinel. That sentinel falls through to `check_deadlock_free` for
/// the precise `Stalled` diagnosis.
///
/// This function never mutates `net` (all three underlying passes clone
/// the net and replay against a private copy) and does not itself panic on
/// any well-formed net: it returns the typed [`PetriAnalysisError`] for
/// every soundness failure, which the driver stringifies into its
/// `Err(String)` channel. (The only transitive panic path is
/// [`Net::fire`]'s `u32` arc-weight/marking overflow guards — genuinely
/// unreachable on v2's small-weight, capacity-bounded nets; an invariant
/// guard, not a valid-input crash.)
///
/// ## What this does and does not prove
///
/// On `Ok(())`: under the single statically-determined firing order
/// v2 picks for this net, no place overflows and the schedule runs to
/// completion. For v2's restricted nets that is equivalent to "bounded
/// and deadlock-free" (PRD §8.4). It is NOT a proof over all possible
/// interleavings — v2 does not have other interleavings, by
/// construction, which is exactly why the replay suffices.
pub fn check_net_sound(net: &Net) -> Result<(), PetriAnalysisError> {
    // Conflict-freedom is checked FIRST, before deriving/replaying the
    // single firing order for boundedness and deadlock. This ordering
    // is deliberate and load-bearing: the boundedness/deadlock passes
    // are sound *only because* the net's firing order is statically
    // determined (PRD §8.4(a)) — they replay ONE arbitrary order and
    // treat it as equivalent to full reachability. If the net actually
    // had a free-choice conflict, that equivalence would be false (the
    // one order they picked could pass while another reachable order
    // overflowed or deadlocked), so their "Ok" would be unsound.
    // Checking the §8.4(a) precondition first means a conflict is
    // reported as a conflict (the real defect) rather than masked by —
    // or masking — a downstream boundedness/deadlock answer computed on
    // an order that should never have been trusted.
    //
    // The firing order is derived ONCE here and shared with the
    // boundedness + deadlock passes (its derivation is purely
    // structural/greedy and does not depend on conflict-freedom). The
    // conflict pass takes the order too, but since TASK-0427 it only USES
    // it in the cap-exceeded fallback (its primary path is a coverability
    // BFS that ignores any single order); handing it in still avoids a
    // redundant `derive_firing_order` call on that fallback path — see
    // TASK-0421/TASK-0427 and the `gate_stays_near_linear_under_large_net`
    // perf pin (the conflict pass short-circuits on every shipping net
    // before either the BFS or the fallback runs).
    let order = derive_firing_order(net);

    check_conflict_free_with_order(net, Some(&order))
        .map_err(PetriAnalysisError::ConflictingChoice)?;

    match check_bounded(net, &order) {
        Ok(()) => {}
        // A genuine capacity overflow, or a malformed firing order
        // (programming error). Both are boundedness answers.
        Err(e @ BoundednessError::CapacityExceeded { .. })
        | Err(e @ BoundednessError::UnknownTransition(_)) => {
            return Err(PetriAnalysisError::Boundedness(e));
        }
        // A stall observed during the boundedness replay. Per
        // `BoundednessError::InvalidFiringOrder`'s own docstring this
        // is "a deadlock-territory problem, not a boundedness one", so
        // we do NOT label it `Boundedness`; we fall through to the
        // deadlock pass below, which diagnoses the identical stall as
        // `DeadlockError::Stalled` (with the stall position) and is
        // mapped to `Deadlock`.
        Err(BoundednessError::InvalidFiringOrder { .. }) => {}
    }

    check_deadlock_free(net, &order).map_err(PetriAnalysisError::Deadlock)?;
    Ok(())
}
