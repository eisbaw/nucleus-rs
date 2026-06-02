//! Cross-crate TEST-ONLY helpers. NOT a public API surface
//! (`#[doc(hidden)]`): these exist solely so integration tests in
//! BOTH `nucleus-compiler/tests/` AND `driver/tests/` can drive the
//! one backend-agnostic pre-mediation pass chain from raw source
//! strings without duplicating the parse/lower/link glue
//! (TASK-0422.01.01). The pass chain proper is single-sourced in
//! [`crate::pipeline::run_pre_mediation_passes`] (TASK-0422.01.01.01) —
//! this helper is a thin source-string wrapper over it.
//!
//! Why a `#[doc(hidden)] pub fn` and not a cargo feature or a shared
//! dev-crate:
//!   - `backend-common` depends on `nucleus-compiler`, so giving
//!     `nucleus-compiler` a dev-dependency back is a cycle risk; a
//!     tiny shared dev-crate would still need an edge from here.
//!   - A `cfg(feature = "test-helpers")` fn enabled via the driver's
//!     `[dependencies]` would (Cargo feature-unification) compile the
//!     helper into the PRODUCTION driver binary.
//!   - A `#[doc(hidden)] pub fn` is always compiled (negligible — it
//!     is a thin composition of already-public passes) but hidden from
//!     rustdoc, and both test crates reach it with ZERO new dependency
//!     edges.

use crate::acfg::ACFG;
use crate::algo::{lower_algo, parse_algo};
use crate::link;
use crate::pipeline::run_pre_mediation_passes;
use crate::sched::{lower_sched, parse_sched};

/// Run the backend-agnostic pre-mediation ACFG pass chain (parse/lower
/// algo + sched -> link -> [the pre-mediation passes]) for one
/// `(algo_src, sched_src)` pair, returning the post-`inject_transfers`
/// ACFG.
///
/// The pass chain itself is NOT open-coded here: it is single-sourced
/// in [`crate::pipeline::run_pre_mediation_passes`], which both this
/// test helper AND the driver's `cmd_build` production path delegate to.
/// Drift between test and production is therefore impossible by
/// construction (TASK-0422.01.01.01 — previously the two were kept
/// identical only by a docstring note, a silent-sibling-defect surface).
/// This helper adds only the test-convenient parts: it takes raw source
/// strings (it parses/lowers/links them) and it DISCARDS the halo
/// advisory bucket the production fn threads out (the driver
/// `nuc_trace!`s it).
///
/// Errors — both from parse/lower/link and from the pre-mediation
/// passes (a [`crate::pipeline::PreMediationError`]) — are mapped via
/// `format!("{e:?}")`. The driver maps the same `PreMediationError` to
/// its own distinct per-pass user-facing strings instead; that mapping
/// lives at the driver call site, not in the shared fn. Callers that
/// need fail-fast (panic) semantics `.expect(...)` the returned
/// `Result`.
///
/// `#[doc(hidden)]`: a cross-crate TEST helper, not a public API
/// surface — see the module docstring for the home-decision rationale.
#[doc(hidden)]
pub fn build_pre_mediation_acfg(algo_src: &str, sched_src: &str) -> Result<ACFG, String> {
    let algo = lower_algo(&parse_algo(algo_src).map_err(|e| format!("{e:?}"))?)
        .map_err(|e| format!("{e:?}"))?;
    let sched = lower_sched(&parse_sched(sched_src).map_err(|e| format!("{e:?}"))?)
        .map_err(|e| format!("{e:?}"))?;
    let linked = link::link(algo, sched).map_err(|e| format!("{e:?}"))?;
    let (acfg, _halo_advisory) =
        run_pre_mediation_passes(&linked).map_err(|e| format!("{e:?}"))?;
    Ok(acfg)
}
