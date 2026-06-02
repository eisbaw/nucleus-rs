//! Cross-crate TEST-ONLY helpers. NOT a public API surface
//! (`#[doc(hidden)]`): these exist solely so integration tests in
//! BOTH `nucleus-compiler/tests/` AND `driver/tests/` can share the
//! one backend-agnostic pre-mediation pass chain without duplicating
//! it (TASK-0422.01.01).
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

use crate::acfg::{build_acfg, ACFG};
use crate::algo::{lower_algo, parse_algo};
use crate::link;
use crate::passes::block_transform::apply_block_transforms;
use crate::passes::halo_inference::apply_halo_inference_partition_aware;
use crate::passes::partition_blocks2d::apply_partition_blocks2d;
use crate::passes::partition_rows::apply_partition_rows;
use crate::passes::partition_workers::apply_partition_workers;
use crate::passes::reuse_inference::apply_reuse_inference;
use crate::passes::sync_inject::inject_syncs;
use crate::passes::transfer_inject::inject_transfers;
use crate::sched::{lower_sched, parse_sched};

/// Run the backend-agnostic pre-mediation ACFG pass chain
/// (parse/lower algo + sched -> link -> build_acfg -> block_transforms
/// -> partition_{workers,rows,blocks2d} -> halo (advisory discarded)
/// -> reuse -> inject_syncs -> inject_transfers) for one
/// `(algo_src, sched_src)` pair, returning the post-`inject_transfers`
/// ACFG.
///
/// MIRRORS the driver's PRODUCTION sequence in
/// `driver/src/main.rs` `cmd_build` (the pre-mediation section, from
/// `build_acfg` through `inject_transfers`, immediately before the
/// `mp-*` `host_mediation_inject`). The pass list AND order were
/// diffed against the driver and confirmed identical when this helper
/// was written (TASK-0422.01.01, cycle-245). This is the test-side
/// single source of that chain; the driver production chain remains a
/// SEPARATE fourth instance (sharing test+production is a larger
/// refactor, deliberately out of scope) — **keep the two in sync**:
/// if you insert a pass before `inject_transfers` in the driver,
/// mirror it here (and vice versa) or the corpus sweeps silently stop
/// exercising the same lowering.
///
/// Errors from every pass are mapped via `format!("{e:?}")` (each
/// pass has a distinct error type); callers that need fail-fast
/// (panic) semantics `.expect(...)` the returned `Result`.
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
    let acfg = build_acfg(&linked).map_err(|e| format!("{e:?}"))?;
    let acfg = apply_block_transforms(&linked, acfg).map_err(|e| format!("{e:?}"))?;
    let acfg = apply_partition_workers(&linked, acfg).map_err(|e| format!("{e:?}"))?;
    let acfg = apply_partition_rows(&linked, acfg).map_err(|e| format!("{e:?}"))?;
    let acfg = apply_partition_blocks2d(&linked, acfg).map_err(|e| format!("{e:?}"))?;
    let (acfg, _adv) =
        apply_halo_inference_partition_aware(&linked, acfg).map_err(|e| format!("{e:?}"))?;
    let acfg = apply_reuse_inference(&linked, acfg).map_err(|e| format!("{e:?}"))?;
    let acfg = inject_syncs(acfg).map_err(|e| format!("{e:?}"))?;
    inject_transfers(&linked, acfg).map_err(|e| format!("{e:?}"))
}
