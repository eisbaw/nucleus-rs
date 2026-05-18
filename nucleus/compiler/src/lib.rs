//! `compiler` crate — Nucleus v2 pre-compiler library surface.
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
pub mod link;
pub mod passes;
pub mod petri;
pub mod sched;

pub use acfg::{
    build_acfg, ACFGNode, DataflowDag, DataflowEdge, NotifyMode, Operation, SyncPlaceholder,
    TransferPolicy, XferPlaceholder, XferRole, ACFG,
};
pub use capabilities::{
    check_schedule_compat, load_capabilities, CapError, CapMismatch, Capabilities,
    NotifyMode as CapNotifyMode, Transport,
};
pub use contract::{check_kernels_contract, ContractError};
pub use error::{ParseError, ParseErrorKind};
pub use event::{DataId, Event, IterTile, IterVar, KernelId, Region, SeqTag, SyncKind, WorkerId};
pub use link::{link, LinkError, LinkedIR, WorkerEntity};
// Petri-net IR (PRD §8). `Arc` is intentionally NOT re-exported at the
// crate root to avoid shadowing `std::sync::Arc` for downstream code;
// reach for it via `compiler::petri::Arc` when you actually need it.
pub use petri::{ArcKind, FireError, Marking, Net, Place, PlaceId, Transition, TransitionId};
pub use passes::sync_inject::inject_syncs;
pub use passes::transfer_inject::inject_transfers;
