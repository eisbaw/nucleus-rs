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

pub mod sync_inject;
pub mod transfer_inject;
