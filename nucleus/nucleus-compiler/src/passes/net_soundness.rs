//! Combined Petri-net soundness gate (TASK-0368, PRD §8.1 / §8.4).
//!
//! This module bundles the two existing analysis passes
//! ([`boundedness`](crate::passes::boundedness) and
//! [`deadlock`](crate::passes::deadlock)) into a single entry point,
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
//! ## Method (and its honest scope)
//!
//! The gate is **exact-replay over one deterministic firing order**,
//! not a general reachability/coverability engine:
//!
//! 1. [`derive_firing_order`] linearises the net into a single
//!    deterministic firing sequence (source order + marking-aware
//!    reordering; see its docstring).
//! 2. [`check_bounded`] replays that order and rejects if any firing
//!    would push a place above its declared capacity.
//! 3. [`check_deadlock_free`] replays that order and rejects if any
//!    step stalls (an input place lacks tokens).
//!
//! This is sound for v2's restricted nets specifically because their
//! firing order is *statically determined* (PRD §8.4: no free-choice,
//! no confusion, no conflicts) and they are *bounded by construction*.
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

use crate::petri::Net;

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
        }
    }
}

impl std::error::Error for PetriAnalysisError {}

// --------------------------------------------------------------------
// check_net_sound
// --------------------------------------------------------------------

/// Run the full Petri-net soundness gate on `net`.
///
/// Derives a deterministic firing order, then checks boundedness, then
/// checks deadlock-freedom against that order. Returns `Ok(())` iff the
/// net passes both.
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
/// This function never panics and never mutates `net`: both underlying
/// passes clone the net and replay against a private copy. The error is
/// typed ([`PetriAnalysisError`]); the driver stringifies it into its
/// `Err(String)` channel.
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
    let order = derive_firing_order(net);

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
