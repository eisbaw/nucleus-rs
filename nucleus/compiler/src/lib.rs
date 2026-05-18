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

pub mod algo;
pub mod contract;
pub mod error;
pub mod link;
pub mod sched;

pub use contract::{check_kernels_contract, ContractError};
pub use error::{ParseError, ParseErrorKind};
pub use link::{link, LinkError, LinkedIR, WorkerEntity};
