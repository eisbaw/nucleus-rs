//! Deadlock analysis pass (TASK-0029, PRD §8.4).
//!
//! Walks a Petri net under a chosen firing order and detects the
//! "structural" deadlock shape: at some step in the order, the next
//! transition cannot fire because one of its input places does not
//! hold enough tokens. PRD §8.4 frames this as a cycle in the global
//! event DAG; for v2 nets whose firing order is statically determined
//! the simulator-replay formulation is exactly equivalent and much
//! cheaper to implement — see the "Why simulator-replay over explicit
//! DAG cycle detection" note below.
//!
//! ## Deadlock in v2
//!
//! PRD §8.2 maps "deadlock" to "a reachable marking where no transition
//! fires". PRD §8.4 sharpens this for v2: per-worker firing order plus
//! `Push`→`Wait` arcs form a DAG, and a cycle in that DAG is a
//! deadlock. Equivalently — and this is the formulation this pass
//! uses — for the specific linearisation that the scheduler chose,
//! deadlock manifests as the next required transition not being
//! enabled. The chosen order is total and statically determined
//! (PRD §8.4: "Statically determined firing order"), so we can simply
//! replay it.
//!
//! ## Why simulator-replay over explicit DAG cycle detection
//!
//! Two valid implementations exist. PRD §8.4 motivates the DAG form;
//! the simulator form falls out of v2's restricted setting (PRD §8.4
//! again: statically determined firing order). Both detect exactly
//! the same set of bugs for v2 nets:
//!
//! 1. **Build the event DAG (intra-worker order + Push→Wait edges)
//!    and look for a cycle.** This is the more direct lowering of
//!    PRD §8.4's description. It needs an explicit edge-extraction
//!    step that the current pipeline does not provide as a first-
//!    class artefact — the per-worker order is implicit in transition
//!    insertion order, and Push→Wait pairing is encoded as a shared
//!    `seq` tag on `Event`s after `petri_to_events`, but the Petri
//!    `Net` itself does not carry the Push→Wait label out.
//!
//! 2. **Replay the linear firing order against the net's initial
//!    marking and observe a stall.** Equivalent for v2's restricted
//!    nets because the linearisation is total. A stall (next
//!    transition not enabled) happens exactly when the would-be
//!    event DAG has a back-edge — the linearisation cannot continue
//!    because some upstream `Push` hasn't deposited its token (it
//!    either does not exist in the net, or is ordered after the
//!    `Wait` in the linearisation). The first stall pinpoints the
//!    offending transition and the deficit place.
//!
//! This pass implements (2). It reuses the same `Net::fire` simulator
//! as `passes::boundedness`, which keeps the analysis library small
//! (PRD §13: ~500 LOC net budget). When a counter-example demands
//! "name the full cycle, not just the stall point", we can layer the
//! DAG form on top later — see `DeadlockError::cycle_hint` doc and
//! the follow-up task that will be filed.
//!
//! ## Errors
//!
//! [`DeadlockError`] carries the offending transition (the one that
//! would not fire), the input place that lacked tokens, how many
//! tokens were present vs. needed, and the marking *immediately
//! before* the stall. The marking-before is the diagnostically useful
//! state — it tells the user "this is what your schedule produced;
//! the next step needs a token here that isn't present".
//!
//! Replay can also fail with `FireError::CapacityExceeded` — that is
//! a *boundedness* violation, not a deadlock. We surface it as
//! [`DeadlockError::CapacityExceeded`] rather than silently passing,
//! so the caller knows the analysis answer is undefined: the run did
//! not deadlock per se, but the net is also not boundedness-clean,
//! and that should have been caught by `check_bounded` upstream.
//!
//! ## Honest limitations
//!
//! - **First stall only.** This pass fails fast on the first deadlock
//!   it observes; it does not enumerate all transitions that would
//!   deadlock from the same starting marking. For v2 v1 that is the
//!   right trade-off (fast feedback, minimal error noise), but a
//!   "list all deadlocks" mode could land later if user feedback
//!   asks for it.
//! - **No structural cycle naming.** The error does *not* name a
//!   cycle in the event DAG; it names the transition that stalled
//!   and the deficit place. When the underlying bug is a missing
//!   `Push` (the case the v2 transfer_inject is currently known to
//!   produce per TASK-0136/0139), the "cycle" is degenerate — there
//!   is no producer at all, so naming a cycle would be misleading.
//!   The deficit-place message is correct in both the missing-producer
//!   and the genuine-cycle cases.
//! - **Exact replay only.** Same caveat as `boundedness`: one firing
//!   sequence is walked. v2's restricted nets (statically determined
//!   firing order, PRD §8.4) make this sound; a future relaxation
//!   that admits true nondeterminism would need a richer reachability
//!   analysis.
//! - **Runtime livelock is out of scope.** This pass catches
//!   *structural* deadlock — schedules where the static order
//!   cannot run to completion. Runtime livelock conditions (e.g. an
//!   `effectful` kernel that never returns) are not addressed; they
//!   cannot be by a static pass.

use crate::petri::{FireError, Marking, Net, PlaceId, TransitionId};

// --------------------------------------------------------------------
// DeadlockError
// --------------------------------------------------------------------

/// Why a [`check_deadlock_free`] call rejected the net.
///
/// All variants carry enough name context to build a user-facing
/// message without re-indexing into the net.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeadlockError {
    /// The firing order stalled: the next transition is not enabled
    /// because an input place holds fewer tokens than the transition
    /// requires.
    ///
    /// This is the central deadlock signal. For v2 with statically
    /// determined firing order (PRD §8.4), "not enabled at this
    /// position in the order" is exactly what a cycle in the global
    /// event DAG manifests as.
    Stalled {
        /// The transition the scheduler tried to fire next.
        transition: TransitionId,
        /// Echoed name for diagnostics.
        transition_name: String,
        /// The input place that does not hold enough tokens.
        place: PlaceId,
        /// Echoed name for diagnostics.
        place_name: String,
        /// Tokens present in `place` at the time of the stall.
        have: u32,
        /// Tokens the transition would need from `place`. Always
        /// `have < need`.
        need: u32,
        /// Marking immediately before the offending step. This is
        /// the state the scheduler must avoid; it tells the user
        /// "execution reached *here* and could go no further".
        marking_before: Marking,
        /// How many transitions in `firing_order` were fired
        /// successfully before the stall. Zero-based, so a stall
        /// on the very first transition reports `0`. Useful for
        /// pointing diagnostics at a specific source position
        /// once the firing-order → source-span map exists.
        position: usize,
    },
    /// The firing order references a transition that the net doesn't
    /// know about. Always a programming error (the caller passed an
    /// id not handed out by this net). Surfaced distinctly so the
    /// caller doesn't conflate it with a real deadlock.
    UnknownTransition(TransitionId),
    /// A firing in the order would push a place above its declared
    /// capacity. That's a boundedness violation, not a deadlock —
    /// `check_bounded` (TASK-0028) is the right place to catch it.
    /// We surface it here so a caller running only the deadlock pass
    /// still gets a clear error rather than a silent Ok.
    CapacityExceeded {
        transition: TransitionId,
        transition_name: String,
        place: PlaceId,
        place_name: String,
        would_be: u32,
        capacity: std::num::NonZeroU32,
    },
}

impl std::fmt::Display for DeadlockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeadlockError::Stalled {
                transition_name,
                place_name,
                have,
                need,
                position,
                ..
            } => write!(
                f,
                "deadlock: firing order stalled at position {} on transition '{}': \
                 place '{}' has {} token(s), needs {}",
                position, transition_name, place_name, have, need
            ),
            DeadlockError::UnknownTransition(t) => {
                write!(
                    f,
                    "deadlock: firing order references unknown transition {:?}",
                    t
                )
            }
            DeadlockError::CapacityExceeded {
                transition_name,
                place_name,
                would_be,
                capacity,
                ..
            } => write!(
                f,
                "deadlock check: firing '{}' would push '{}' to {} (capacity {}); \
                 this is a boundedness violation, not a deadlock",
                transition_name, place_name, would_be, capacity
            ),
        }
    }
}

impl std::error::Error for DeadlockError {}

// --------------------------------------------------------------------
// check_deadlock_free
// --------------------------------------------------------------------

/// Replay `firing_order` against `net`'s initial marking and verify
/// that every step is enabled in turn.
///
/// The function does not mutate `net`; it clones the net and fires
/// against a private copy. The original is read only for name lookup
/// in error payloads.
///
/// Returns `Ok(())` if every transition in `firing_order` fires
/// successfully. Returns the first stall otherwise. Reporting the
/// first stall (rather than enumerating all of them) keeps the error
/// surface small and matches "fail fast and verbosely".
///
/// ## Determinism
///
/// `Net::fire` and `Marking` iteration are both deterministic
/// (BTreeMap-backed, see `petri.rs`), so for the same `(net,
/// firing_order)` this function returns the same `Result` every
/// call, including the same `marking_before` snapshot.
pub fn check_deadlock_free(
    net: &Net,
    firing_order: &[TransitionId],
) -> Result<(), DeadlockError> {
    // Work on a clone — callers don't observe state mutation.
    let mut sim = net.clone();
    sim.reset_to_initial();

    for (position, &tid) in firing_order.iter().enumerate() {
        // Snapshot the marking *before* the firing. Kept on the
        // stack; only retained in the error payload on failure.
        let marking_before = sim.current_marking.clone();

        match sim.fire(tid) {
            Ok(_) => {}
            Err(FireError::UnknownTransition(t)) => {
                return Err(DeadlockError::UnknownTransition(t));
            }
            Err(FireError::NotEnabled {
                transition,
                place,
                have,
                need,
            }) => {
                return Err(DeadlockError::Stalled {
                    transition,
                    transition_name: name_of_transition(net, transition),
                    place,
                    place_name: name_of_place(net, place),
                    have,
                    need,
                    marking_before,
                    position,
                });
            }
            Err(FireError::CapacityExceeded {
                transition,
                place,
                would_be,
                capacity,
            }) => {
                return Err(DeadlockError::CapacityExceeded {
                    transition,
                    transition_name: name_of_transition(net, transition),
                    place,
                    place_name: name_of_place(net, place),
                    would_be,
                    capacity,
                });
            }
        }
    }

    Ok(())
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
