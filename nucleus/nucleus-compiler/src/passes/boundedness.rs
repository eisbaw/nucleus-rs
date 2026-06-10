//! Boundedness analysis pass (TASK-0028, PRD §8.2).
//!
//! Walks a Petri net under a chosen firing order and checks that no
//! reachable marking exceeds any place's declared capacity.
//!
//! ## What boundedness means in v2
//!
//! PRD §8.2 maps "boundedness" to "every place stays within its
//! declared capacity". Concretely, for every place `p` with capacity
//! `Some(c)`, no firing of the chosen linear order should ever raise
//! `p`'s token count above `c`.
//!
//! This is a structural check, not a symbolic one: v2 nets are bounded
//! by construction (PRD §8.4) and the firing order is statically
//! determined (PRD §8.4 again). So "is the marking bounded?" reduces
//! to "does the chosen linear order respect every place's capacity?",
//! which is decidable by a single replay from the initial marking.
//!
//! The check is intentionally *exact-replay*, not symbolic
//! reachability. v2's restricted nets do not need general-purpose
//! coverability or Karp-Miller; they have a single linear firing
//! schedule which we walk in order.
//!
//! ## Firing order
//!
//! The function accepts a firing order as an explicit input. This
//! matches PRD §8.6's "v2 picks a deterministic greedy order ... and
//! validates that order against the net properties above" — the
//! linearisation is computed once (here, by [`derive_firing_order`])
//! and then validated against each property in turn.
//!
//! For convenience this module also exposes [`derive_firing_order`]
//! that produces a deterministic order by walking a virtual marking:
//! at each step it picks the first transition in source order that is
//! *firable* under the current marking (enabled AND not capacity-
//! overflowing). The chain of per-worker control places (built by
//! `acfg_to_petri` — see TASK-0026) guarantees that this strategy
//! advances each worker through its own per-worker linear order, and
//! that cross-worker transitions (Sync, Push/Wait pairs) only become
//! enabled once all upstream per-worker tokens are present.
//!
//! The marking-aware step is what makes the order respect *initial
//! marking* — when a buffer place is pre-marked at capacity by a
//! `pipeline=D` loop (TASK-0134), the producer Push at the head of
//! the source order is not firable (it would overflow), so the
//! algorithm naturally pulls the matching consumer Wait forward.
//! For nets without nonzero *buffer* initial markings (the seeded `=1`
//! head token on each worker's first control place aside) the source-order
//! tiebreak makes the result identical to plain insertion order, so
//! existing non-pipelined fixtures see no change (TASK-0213).
//!
//! ## Errors
//!
//! [`BoundednessError`] carries the offending place, the transition
//! that would overflow it, the post-firing token count that would
//! result, the declared capacity, and the marking *immediately before*
//! the offending firing. The marking before is more diagnostically
//! useful than the marking after: it tells the user the exact state
//! the schedule needs to avoid.
//!
//! Replay can also fail with `FireError::NotEnabled` or
//! `FireError::UnknownTransition` — neither is a boundedness failure
//! but both render the boundedness question undefined. They are
//! surfaced as [`BoundednessError::InvalidFiringOrder`] / `UnknownTransition`
//! so the caller cannot mistake them for "all fine".
//!
//! ## Honest limitations
//!
//! - **Exact replay only.** This pass walks one concrete firing
//!   sequence. It does not enumerate alternative interleavings. v2's
//!   restricted nets (statically determined firing order, PRD §8.4)
//!   make this sound; a future relaxation that admits true
//!   nondeterminism would need a coverability check, not this.
//! - **Unbounded places (`capacity = None`) are accepted.** They are
//!   intended for analysis fixtures (per `petri::Place::capacity`
//!   docs). Production nets emitted by `acfg_to_net` always set
//!   `Some(_)`, so this path is dormant in the real pipeline.
//! - **No minimum-N suggestion.** When a capacity-N place overflows,
//!   we report the would-be count but do not synthesise a "use
//!   buffer=K instead" hint. That belongs in a downstream diagnostic
//!   layer that knows the schedule-source location of the relevant
//!   `transfer DATA : buffer=N`.

use std::num::NonZeroU32;

use crate::petri::{FireError, Marking, Net, PlaceId, TransitionId};
// `Marking` is used both in `BoundednessError::CapacityExceeded`'s
// payload type and for the borrowed-replay state in `check_bounded` /
// `derive_firing_order` (TASK-0455.10 cut the whole-net clone).

// --------------------------------------------------------------------
// BoundednessError
// --------------------------------------------------------------------

/// Why a [`check_bounded`] call rejected the net.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoundednessError {
    /// A firing would push a place above its declared capacity.
    /// `place_name` and `transition_name` are echoed from the net so
    /// callers can build user-facing messages without re-indexing.
    CapacityExceeded {
        place: PlaceId,
        place_name: String,
        transition: TransitionId,
        transition_name: String,
        /// Marking immediately before the offending firing.
        marking_before: Marking,
        /// Token count the place would reach if the firing committed.
        would_be: u32,
        /// The place's declared capacity (>= 1; `NonZeroU32`).
        capacity: NonZeroU32,
    },
    /// The firing order references a transition that the net doesn't
    /// know about. Always a programming error (the caller passed an
    /// id not handed out by this net); surfaced as a distinct variant
    /// so callers don't conflate it with a real capacity violation.
    UnknownTransition(TransitionId),
    /// The firing order is not a legal interleaving — a transition
    /// was requested whose input places held too few tokens. This
    /// is a deadlock-territory problem, not a boundedness one, but
    /// it makes "is the net bounded?" undefined so we surface it
    /// rather than silently returning Ok.
    InvalidFiringOrder {
        transition: TransitionId,
        transition_name: String,
        place: PlaceId,
        place_name: String,
        have: u32,
        need: u32,
    },
}

impl std::fmt::Display for BoundednessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BoundednessError::CapacityExceeded {
                place_name,
                transition_name,
                would_be,
                capacity,
                ..
            } => write!(
                f,
                "boundedness: firing transition '{}' would push place '{}' to {} tokens (capacity {})",
                transition_name, place_name, would_be, capacity
            ),
            BoundednessError::UnknownTransition(t) => {
                write!(f, "boundedness: firing order references unknown transition {:?}", t)
            }
            BoundednessError::InvalidFiringOrder {
                transition_name,
                place_name,
                have,
                need,
                ..
            } => write!(
                f,
                "boundedness: firing order is not legal: transition '{}' needs {} token(s) in place '{}', has {}",
                transition_name, need, place_name, have
            ),
        }
    }
}

impl std::error::Error for BoundednessError {}

// --------------------------------------------------------------------
// check_bounded
// --------------------------------------------------------------------

/// Replay `firing_order` against `net`'s initial marking and verify
/// that no place exceeds its declared capacity at any step.
///
/// The function does not mutate `net`; it clones the marking and
/// fires transitions against a private copy. The net itself is read
/// only to look up names for error reporting.
///
/// Returns `Ok(())` if every firing in `firing_order` respects every
/// place's capacity. Returns the first violation otherwise — we
/// report the *first* problem rather than enumerating all of them,
/// which keeps the error surface small and matches the "fail fast and
/// verbosely" rule.
pub fn check_bounded(net: &Net, firing_order: &[TransitionId]) -> Result<(), BoundednessError> {
    // Replay on a borrowed `&net` plus an owned copy of the initial
    // marking. We do NOT clone the whole net (places + transitions +
    // arcs): the replay only ever mutates the marking, so cloning just
    // the `Marking` is sufficient and cuts the gate's per-pass memory
    // from "one net" to "one marking" (TASK-0455.10). `Net::reset_to_initial`
    // copies `initial_marking` into `current_marking`; we clone the same
    // source directly.
    let mut marking = net.initial_marking.clone();

    // Build the per-transition arc index ONCE (O(A)) so each fire is
    // O(deg(t)) instead of an all-arcs scan (TASK-0377). The index is
    // keyed by TransitionId and built from `net`.
    let index = net.build_arc_index();

    for &tid in firing_order {
        // `fire_marking` leaves `marking` unmutated on failure (all
        // failure modes are checked before any token moves), so on the
        // CapacityExceeded arm `marking` IS the marking *before* the
        // offending firing — clone it lazily there rather than every
        // step (TASK-0377).
        match net.fire_marking(tid, &mut marking, &index) {
            Ok(()) => {}
            Err(FireError::UnknownTransition(t)) => {
                return Err(BoundednessError::UnknownTransition(t));
            }
            Err(FireError::NotEnabled {
                transition,
                place,
                have,
                need,
            }) => {
                return Err(BoundednessError::InvalidFiringOrder {
                    transition,
                    transition_name: name_of_transition(net, transition),
                    place,
                    place_name: name_of_place(net, place),
                    have,
                    need,
                });
            }
            Err(FireError::CapacityExceeded {
                transition,
                place,
                would_be,
                capacity,
            }) => {
                return Err(BoundednessError::CapacityExceeded {
                    place,
                    place_name: name_of_place(net, place),
                    transition,
                    transition_name: name_of_transition(net, transition),
                    marking_before: marking.clone(),
                    would_be,
                    capacity,
                });
            }
        }
    }

    Ok(())
}

// --------------------------------------------------------------------
// derive_firing_order
// --------------------------------------------------------------------

/// Produce a deterministic firing order from a net built by
/// [`crate::passes::acfg_to_petri::acfg_to_net`].
///
/// ## Algorithm
///
/// At each step, replay against a virtual marking (initialised from
/// `net.initial_marking`) and pick the *first* transition in source
/// order that is firable — i.e. enabled (every input place has enough
/// tokens) AND would not overflow any output place's capacity. Fire
/// it, drop it from the remaining set, repeat. Stop when either:
///
/// 1. every transition has been fired (success — the returned order
///    is a legal firing sequence), or
/// 2. no remaining transition is firable. We append the remaining
///    transitions in source order so that [`check_bounded`] /
///    [`check_deadlock_free`](crate::passes::deadlock::check_deadlock_free) still surface a precise diagnostic at
///    the stall point (`BoundednessError::CapacityExceeded` or
///    `BoundednessError::InvalidFiringOrder` /
///    `DeadlockError::Stalled`) rather than silently truncating.
///
/// ## Why this (vs. plain insertion order)
///
/// `acfg_to_net` walks the ACFG depth-first in source order, so
/// transition ids are assigned in source order and per-worker control
/// chains discharge most ordering constraints structurally. For nets
/// without nonzero *buffer* initial markings, source order *is* a legal
/// firing sequence, and the algorithm above degenerates to "fire
/// transitions in id order" — so all pre-TASK-0213 fixtures see no
/// change. (The seeded `=1` head token on each worker's first control
/// place does NOT count: it is exactly what makes the chain firable in
/// id order. The only initial markings that perturb the order are the
/// `pipeline=D` buffer pre-marks below.)
///
/// What pure insertion order does NOT cope with is a buffer place
/// pre-marked at capacity, where source-order's first transition is
/// a producer that would overflow. The algorithm above will pull a
/// later consumer transition forward when one is firable, producing
/// a legal interleaving.
///
/// **Why this defensive layer is kept (TASK-0219).** With TASK-0213's
/// path-2 elision in `acfg_to_petri`, every net the in-tree pipeline
/// produces has source-order legal directly — so the marking-aware
/// reorder above and the stuck-state fallback below are NOT exercised
/// by any compiler-built fixture today. The function is `pub` and
/// accepts ANY [`Net`], including hand-built ones with soft
/// constraints (legal interleaving exists but isn't source-order).
/// Removing path-1 would punish those callers (incl. analysis-only
/// experiments) without saving meaningful complexity. Path-1 + the
/// stuck-state fallback are pinned by two synthetic-net unit tests in
/// `tests/boundedness.rs` (`derive_firing_order_reorders_under_initial_marking_pressure`
/// and `derive_firing_order_appends_stuck_leftovers_so_check_bounded_diagnoses`).
/// Defensive code WITH tests is acceptable; dead-code-with-no-test was
/// the original TASK-0219 finding — closed.
///
/// **Honest note on the example-13 fixture**: pipeline=D's *first*
/// expression of "the buffer has D head-start credits" is the
/// `pipeline_depth_for_seq` → `initial_marking = D` mapping
/// (TASK-0134). The *second*, complementary expression — eliding the
/// first D producer Push TtoP arcs in `acfg_to_petri::emit_xfer`
/// (TASK-0213 path 2) — is what makes source-order legal on the
/// example-13 net directly. Without path 2, sync_inject's barrier
/// between Push and Wait creates a structural cycle that this
/// path-1 reordering alone cannot resolve (the consumer Wait is
/// blocked behind the barrier). Path 1 here is the general fallback
/// for nets whose source-order isn't legal even after path-2
/// elision — currently no in-tree fixture exercises it; see
/// TASK-0218 for the underlying sync_inject limitation.
///
/// We accept any [`Net`] here, not only nets built by `acfg_to_net`.
/// For other producers the function still returns a deterministic
/// order; if the net is genuinely deadlocked or boundedness-broken
/// the downstream passes surface it cleanly.
///
/// ## Determinism
///
/// - Source order is iteration over a `Vec<Transition>`, indexed
///   `0..N`. No hash-map iteration.
/// - The virtual marking lives in a cloned `Net` and so reuses the
///   `BTreeMap`-backed `Marking` (see `petri.rs`).
/// - The "first firable" tiebreak is by source index, the cheapest
///   deterministic choice. For the same `net` the output is
///   byte-identical across runs.
pub fn derive_firing_order(net: &Net) -> Vec<TransitionId> {
    let total = net.transitions.len();
    let mut order = Vec::with_capacity(total);
    let mut fired: Vec<bool> = vec![false; total];

    // Replay against an owned copy of the initial marking so we don't
    // touch the caller's net. Cloning the `Marking` (not the whole net)
    // is sufficient because the firability probe below only mutates the
    // marking (TASK-0455.10). Starting from `net.initial_marking` is
    // exactly what `reset_to_initial` would copy in (TASK-0134's
    // `initial_marking = D` lands here).
    let mut marking = net.initial_marking.clone();

    // Build the per-transition arc index ONCE (O(A)) so the firability
    // oracle below is O(deg(t)) per probe instead of an all-arcs scan
    // (TASK-0377).
    let index = net.build_arc_index();

    // Scan cursor: `start` is the lowest index still `false` in
    // `fired`. Each outer iteration scans from `start` rather than 0,
    // which removes the O(T²) re-skipping of the already-fired
    // contiguous prefix (TASK-0377 tertiary win). This is BYTE-IDENTICAL
    // to scanning from 0: indices below `start` are all `fired == true`,
    // which the old `if fired[idx] { continue; }` would skip anyway, so
    // the "first firable un-fired transition" chosen is unchanged.
    let mut start = 0usize;

    'outer: loop {
        // Scan in source order from the cursor; the first firable
        // un-fired transition wins. `Net::fire_in_place` commits on
        // success and leaves the marking alone on failure, so we can
        // use it as the firability oracle directly without an extra
        // `enabled_transitions` call.
        for (idx, t) in net.transitions.iter().enumerate().skip(start) {
            if fired[idx] {
                continue;
            }
            if net.fire_marking(t.id, &mut marking, &index).is_ok() {
                order.push(t.id);
                fired[idx] = true;
                // Advance the cursor past any now-contiguous fired
                // prefix so future scans skip it in O(1).
                while start < total && fired[start] {
                    start += 1;
                }
                continue 'outer;
            }
        }
        // No remaining transition is firable. Either we are done
        // (every fired[i] is true) or the net is structurally stuck
        // and downstream passes will diagnose. In the stuck case,
        // append leftovers in source order so the diagnostic points
        // at the first stuck transition rather than silently
        // truncating the trace.
        break;
    }

    if order.len() < total {
        for (idx, t) in net.transitions.iter().enumerate() {
            if !fired[idx] {
                order.push(t.id);
            }
        }
    }

    order
}

// --------------------------------------------------------------------
// Helpers
// --------------------------------------------------------------------

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
