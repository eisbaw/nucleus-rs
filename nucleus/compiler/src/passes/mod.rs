//! Compiler passes that consume and produce an [`crate::acfg::ACFG`].
//!
//! Each submodule is one self-contained transformation. The pipeline
//! order is fixed by the PRD §5 diagram:
//!
//! 1. [`sync_inject`] — TASK-0017. Insert barrier syncs where
//!    control-flow joins require it.
//! 2. (future) transfer injection — TASK-0018.
//! 3. (future) Petri-net construction — later milestones.
//!
//! Passes are pure functions `ACFG -> ACFG` so they compose with
//! function-composition rules and tests can pipe them in any order.

pub mod sync_inject;
