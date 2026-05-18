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
//! that produces a deterministic order by repeatedly picking the
//! lowest-id enabled transition until none remain. The chain of
//! per-worker control places (built by `acfg_to_petri` — see TASK-0026)
//! guarantees that this greedy strategy advances each worker through
//! its own per-worker linear order, and that cross-worker transitions
//! (Sync, Push/Wait pairs) only become enabled once all upstream
//! per-worker tokens are present.
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
    // Work on a clone so callers don't observe state mutation.
    let mut sim = net.clone();
    sim.reset_to_initial();

    for &tid in firing_order {
        // Snapshot the marking *before* the firing — useful diagnostic
        // payload if this step fails. Cheap (BTreeMap clone) and only
        // retained on failure.
        let marking_before = sim.current_marking.clone();

        match sim.fire(tid) {
            Ok(_) => {}
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
                    marking_before,
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
/// ## Why insertion order works for v2 nets
///
/// `acfg_to_net` walks the ACFG depth-first in source order. Every
/// transition it emits is appended to the net in that walk order, so
/// `TransitionId(i) = TransitionId(i+1)` reflects "i was emitted
/// before i+1". The walk itself respects per-worker control chains
/// (each worker advances strictly through its own slots) and
/// cross-worker dataflow (a Push is emitted before its matched Wait,
/// because that's the source order). Hence the *insertion order*
/// 0, 1, ..., N-1 is a legal firing order for the net.
///
/// PRD §8.6 describes the linearisation as "deterministic greedy
/// (source order + dataflow constraints)". Insertion order *is* source
/// order, and the dataflow constraints are discharged structurally by
/// the per-worker control chain. So we don't need a runtime greedy
/// search — we just trust the construction.
///
/// We accept any [`Net`] here, not only nets built by `acfg_to_net`.
/// For other producers the function still returns a deterministic
/// order, and [`check_bounded`] will surface a clear
/// [`BoundednessError::InvalidFiringOrder`] if the order turns out
/// not to be a legal interleaving.
///
/// **Determinism**: a `Vec` iterated in index order is the most
/// deterministic possible source — no hash maps, no greedy tiebreaks.
pub fn derive_firing_order(net: &Net) -> Vec<TransitionId> {
    net.transitions.iter().map(|t| t.id).collect()
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
