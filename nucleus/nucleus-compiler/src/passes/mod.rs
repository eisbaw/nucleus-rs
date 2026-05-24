//! Compiler passes that consume and produce an [`crate::acfg::ACFG`].
//!
//! Each submodule is one self-contained transformation. The pipeline
//! order is fixed by the PRD §5 diagram:
//!
//! 1. [`sync_inject`] — TASK-0017. Insert barrier syncs where
//!    control-flow joins require it.
//! 2. [`transfer_inject`] — TASK-0018. Insert matched Push/Wait
//!    placeholders for every dataflow edge that crosses workers.
//! 3. (future) Petri-net construction — later milestones.
//!
//! Passes are pure functions `ACFG -> ACFG` (or `(&LinkedIR, ACFG) ->
//! ACFG` where the link-pass output is also needed; see
//! [`transfer_inject::inject_transfers`]) so they compose with
//! function-composition rules and tests can pipe them in any order.

pub mod acfg_to_petri;
pub mod block_transform;
pub mod boundedness;
pub mod deadlock;
// TASK-0260 Stage 1: halo region inference from kernel access patterns.
// Runs AFTER `apply_partition_blocks2d` (driver pass order), AFTER
// `build_acfg` (needs name_iter_vars / name_kernels). Pure +
// observationally-inert in Stage 1: writes `ACFG::halo_widths` for
// Stage 2 (transfer_inject extension, TASK-0263) to consume.
pub mod halo_inference;
// TASK-0052.02: real-time `check loop V : latency_max=T` projection.
// Runs AFTER `petri_to_events`, BEFORE backend codegen — see module
// docstring for the dependency rationale.
pub mod inject_check_frames;
pub mod partition_blocks2d;
pub mod partition_rows;
pub mod partition_workers;
pub mod petri_to_events;
pub mod sync_inject;
pub mod transfer_inject;
