//! Backend-agnostic pre-mediation pass orchestration.
//!
//! The pre-mediation pass chain — the sequence of ACFG-construction,
//! block-transform, partition, halo-inference, reuse-inference, and
//! sync/transfer-injection passes that runs on every `nucleus build`
//! *before* any backend-specific host-mediation / host-data-relay
//! injection — is the SINGLE SOURCE OF TRUTH for how a linked program
//! is lowered to a pre-mediation ACFG.
//!
//! Historically this chain was open-coded in four places: the driver's
//! `cmd_build` production path plus three integration-test copies
//! (TASK-0422.01.01 consolidated the three test copies into
//! `crate::test_support::build_pre_mediation_acfg`). The four copies
//! were kept identical only by a docstring note — a textbook
//! silent-sibling-defect surface: inserting a pass before
//! `inject_transfers` in one copy and not the others would silently
//! diverge the corpus sweeps from the real driver lowering with no gate
//! to catch it.
//!
//! [`run_pre_mediation_passes`] is the structural fix (TASK-0422.01.01.01):
//! ONE production function that BOTH the driver and the test helper call.
//! Drift is now impossible by construction — there is one pass list, in
//! one place. The two callers differ only in how they MAP the typed
//! [`PreMediationError`] to a user-facing message (the driver uses a
//! distinct `Display` string per pass; the test helper uses a uniform
//! `{:?}`) and in whether they thread the halo advisory bucket out
//! (the driver `nuc_trace!`s it; the helper discards it).

use crate::acfg::{build_acfg, BuildAcfgError, ACFG};
use crate::link::LinkedIR;
use crate::passes::block_transform::{apply_block_transforms, BlockTransformError};
use crate::passes::halo_inference::{apply_halo_inference_partition_aware, HaloInferenceError};
use crate::passes::partition_blocks2d::{apply_partition_blocks2d, PartitionBlocks2dError};
use crate::passes::partition_rows::{apply_partition_rows, PartitionRowsError};
use crate::passes::partition_workers::{apply_partition_workers, PartitionError};
use crate::passes::reuse_inference::{apply_reuse_inference, ReuseInferenceError};
use crate::passes::sync_inject::{inject_syncs, SyncInjectError};
use crate::passes::transfer_inject::inject_transfers;
use crate::passes::transfer_inject::TransferInjectError;

/// Typed failure of one pre-mediation pass.
///
/// One variant per pass in [`run_pre_mediation_passes`], each wrapping
/// that pass's own error type so the caller can recover the exact cause.
/// Deliberately has NO `Display` impl: the user-facing message is the
/// CALLER's policy (the driver maps each variant to a distinct
/// per-pass string; the test helper uses `{:?}` via the derived
/// [`Debug`]). Baking the driver's strings in here would re-create the
/// drift it is meant to remove — and the test helper does not want
/// them.
///
/// Deliberately NOT `#[non_exhaustive]`: the driver's `cmd_build`
/// matches every variant to assign a distinct user-facing error string.
/// If a pass is added to [`run_pre_mediation_passes`] (a new variant),
/// the driver's match must FAIL TO COMPILE so the implementer is forced
/// to give the new pass a user-facing message rather than silently
/// falling into a catch-all `_` arm. That compile error is the
/// drift-prevention guarantee this enum exists for — a `#[non_exhaustive]`
/// wildcard would defeat it (silent-sibling-defect class).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreMediationError {
    /// `build_acfg` failed (dataflow / structural ACFG construction).
    AcfgBuild(BuildAcfgError),
    /// `apply_block_transforms` failed (`block=` directive rewrite).
    BlockTransform(BlockTransformError),
    /// `apply_partition_workers` failed (`partition=workers` rewrite).
    PartitionWorkers(PartitionError),
    /// `apply_partition_rows` failed (`partition=rows` rewrite).
    PartitionRows(PartitionRowsError),
    /// `apply_partition_blocks2d` failed (`partition=blocks2d` rewrite).
    PartitionBlocks2d(PartitionBlocks2dError),
    /// `apply_halo_inference_partition_aware` raised a FATAL halo error
    /// (a non-affine / strided index under a `partition=` iv). Advisory
    /// (non-partition-scoped) halo errors are NOT this variant — they
    /// are returned in the `Ok` advisory bucket.
    HaloInference(HaloInferenceError),
    /// `apply_reuse_inference` failed (strict: any typed reuse error).
    ReuseInference(ReuseInferenceError),
    /// `inject_syncs` failed (sync-placeholder injection).
    SyncInject(SyncInjectError),
    /// `inject_transfers` failed (Push/Wait transfer injection).
    TransferInject(TransferInjectError),
}

/// Run the backend-agnostic pre-mediation ACFG pass chain on one
/// already-linked program, returning the post-`inject_transfers` ACFG
/// plus the halo-inference ADVISORY bucket (non-fatal halo errors whose
/// affected iv carries no `partition=` directive in scope — the
/// `transfer_inject` halo consumer will not fire on them, so lowering
/// proceeds; the driver `nuc_trace!`s each, the test helper discards
/// them).
///
/// The pass sequence, in order, is:
///
/// 1. [`build_acfg`] — construct the ACFG from the linked dataflow.
/// 2. [`apply_block_transforms`] — `block=` directive rewrite
///    (identity for schedules with no `block=`).
/// 3. [`apply_partition_workers`] — `partition=workers` per-worker
///    range override on the sidecar.
/// 4. [`apply_partition_rows`] — `partition=rows` row-band rewrite.
/// 5. [`apply_partition_blocks2d`] — `partition=blocks2d` 2D-grid
///    rewrite.
/// 6. [`apply_halo_inference_partition_aware`] — infer per-(KernelId,
///    IterVar) halo widths; FATAL on a non-affine index under a
///    `partition=` iv, ADVISORY otherwise (TASK-0275 policy B).
/// 7. [`apply_reuse_inference`] — infer per-(IterVar, DataId, axis)
///    reuse delay-line slots; STRICT (any typed error is fatal,
///    TASK-0271).
/// 8. [`inject_syncs`] — insert sync placeholders.
/// 9. [`inject_transfers`] — insert matched Push/Wait pairs.
///
/// The partition passes (3-5) target disjoint `IterVar` keys by grammar
/// construction (at most one `partition=` per loop), so their relative
/// order is observationally irrelevant; the order here matches the
/// driver's for diagnostic clarity.
///
/// This is the ONE production definition of the chain. The driver's
/// `cmd_build` and `crate::test_support::build_pre_mediation_acfg` both
/// delegate here, so the chain cannot drift between test and production.
pub fn run_pre_mediation_passes(
    linked: &LinkedIR,
) -> Result<(ACFG, Vec<HaloInferenceError>), PreMediationError> {
    let acfg = build_acfg(linked).map_err(PreMediationError::AcfgBuild)?;
    let acfg =
        apply_block_transforms(linked, acfg).map_err(PreMediationError::BlockTransform)?;
    let acfg =
        apply_partition_workers(linked, acfg).map_err(PreMediationError::PartitionWorkers)?;
    let acfg = apply_partition_rows(linked, acfg).map_err(PreMediationError::PartitionRows)?;
    let acfg = apply_partition_blocks2d(linked, acfg)
        .map_err(PreMediationError::PartitionBlocks2d)?;
    let (acfg, halo_advisory) = apply_halo_inference_partition_aware(linked, acfg)
        .map_err(PreMediationError::HaloInference)?;
    let acfg =
        apply_reuse_inference(linked, acfg).map_err(PreMediationError::ReuseInference)?;
    let acfg = inject_syncs(acfg).map_err(PreMediationError::SyncInject)?;
    let acfg =
        inject_transfers(linked, acfg).map_err(PreMediationError::TransferInject)?;
    Ok((acfg, halo_advisory))
}
