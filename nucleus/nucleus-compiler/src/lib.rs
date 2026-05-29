//! `nucleus-compiler` crate — Nucleus v2 pre-compiler library surface.
//!
//! At M0 this exposes the algorithm- and schedule-sublanguage parsers.
//! Subsequent milestones will add typechecking, AlgoIR/SchedIR
//! lowering, the link step, and codegen. See `nuc-nucleus/PRD.md`
//! §12.2.
//!
//! The public surface is intentionally minimal: re-export `algo`,
//! `sched`, and the shared `error` types. Internal modules stay
//! private until a caller needs them.

pub mod acfg;
pub mod algo;
pub mod capabilities;
pub mod contract;
pub mod error;
pub mod event;
pub mod event_validate;
pub mod link;
pub mod name_tables;
pub mod passes;
pub mod petri;
pub mod sched;
pub mod sidecar;
// Per-node source-span wrapper shared by the algorithm AST (TASK-0082)
// and the schedule AST (TASK-0086). Promoted from the former
// algo-local `algo::span` so both sub-language ASTs build on one
// implementation of the load-bearing "PartialEq ignores span"
// semantics — see `span.rs` for the share-vs-duplicate rationale.
pub mod span;
// Zero-dep, NUC_TRACE-gated diagnostics facility. The `nuc_trace!`
// macro is `#[macro_export]`ed at the crate root; this module holds
// its sink + the decision rationale (TASK-0154).
pub mod trace;

pub use acfg::{
    build_acfg, ACFGNode, BuildAcfgError, DataAccess, DataflowDag, DataflowEdge, LoopBoundEnd,
    NotifyMode, Operation, SyncPlaceholder, TransferPolicy, XferPlaceholder, XferRole, ACFG,
};
pub use capabilities::{
    check_schedule_compat, load_capabilities, CapError, CapMismatch, Capabilities,
    NotifyMode as CapNotifyMode, Transport,
};
pub use contract::{check_kernels_contract, ContractError};
pub use error::{ParseError, ParseErrorKind, ParseErrors};
pub use event::{
    ArgBinding, CheckFrame, DataId, DataSlice, Event, FireBinding, IterTile, IterVar, KernelId,
    Region, SeqTag, SyncKind, SyncTag, ViolationKind, WorkerId,
};
pub use event_validate::{
    validate_event_lists, validate_event_lists_strict_per_worker, EventValidationError,
};
pub use link::{link, LinkError, LinkErrorKind, LinkErrorSource, LinkedIR, WorkerEntity};
// Petri-net IR (PRD §8). `Arc` is intentionally NOT re-exported at the
// crate root to avoid shadowing `std::sync::Arc` for downstream code;
// reach for it via `nucleus_compiler::petri::Arc` when you actually need it.
pub use name_tables::NameTables;
pub use passes::acfg_to_petri::acfg_to_net;
pub use passes::block_transform::{apply_block_transforms, BlockTransformError};
pub use passes::boundedness::{check_bounded, derive_firing_order, BoundednessError};
pub use passes::common::affine_decompose;
pub use passes::deadlock::{check_deadlock_free, DeadlockError};
pub use passes::halo_inference::{
    apply_halo_inference, apply_halo_inference_advisory, apply_halo_inference_partition_aware,
    HaloInferenceError,
};
// TASK-0329.01.02 slice 2: host-mediated data-relay injection for mp-tcp-event.
pub use passes::host_data_relay_inject::apply_host_data_relay_inject;
pub use passes::host_mediation_inject::apply_host_mediation_inject;
pub use passes::inject_check_frames::inject_check_frames;
pub use passes::partition_blocks2d::{apply_partition_blocks2d, PartitionBlocks2dError};
pub use passes::partition_rows::{apply_partition_rows, PartitionRowsError};
pub use passes::partition_workers::{apply_partition_workers, PartitionError};
pub use passes::petri_to_events::{acfg_to_events, petri_to_events};
pub use passes::reuse_inference::{
    apply_reuse_inference, apply_reuse_inference_advisory, ReuseInferenceError, ReuseSlot,
};
// TASK-0329.01.01 slice 1: safe push-before-wait reordering for mp-tcp-event.
pub use passes::safe_push_reorder::apply_safe_push_reorder;
pub use passes::sync_inject::{inject_syncs, SyncInjectError};
pub use passes::transfer_inject::inject_transfers;
pub use petri::{ArcKind, FireError, Marking, Net, Place, PlaceId, Transition, TransitionId};
pub use sidecar::{build_sidecar, ConstValue, KernelSig, LoopBound, NameSidecar, SidecarError};
