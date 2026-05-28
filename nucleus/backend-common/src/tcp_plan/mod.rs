//! Shared multi-process emit substrate for the sync-TCP backends
//! (mp-tcp-bufsync, mp-tcp-poll). TASK-0044.02.03.
//!
//! # Why this module exists
//!
//! `mp-tcp-poll` was created (cycle 195, TASK-0044.02.02) by copying
//! `mp-tcp-bufsync`'s `src/{encode.rs, walkers.rs, plan/*}` substrate
//! verbatim and swapping the wait primitive (blocking `recv` →
//! nonblocking poll + `yield_now`). Two consumers proved the
//! abstraction well-shaped, so this module lifts the ~1.4k LoC of
//! behaviour-bearing plan/walkers/encode logic into a single source of
//! truth parameterised over the [`WirePrimitives`] trait — the same
//! "fail fast, prove a second consumer, then lift" precedent as the
//! original backend-common lift (TASK-0244).
//!
//! Each backend crate now reduces to a thin shim: a `WirePrimitives`
//! impl plus a `type Plan<'a> = backend_common::tcp_plan::Plan<'a,
//! ThisWire>` alias and re-exports of the shared walkers/encode helpers.
//!
//! # Variation surface
//!
//! [`WirePrimitives`] is the SOLE axis of variation. See its docstring
//! for the complete enumeration; in short: the three emitted wire-call
//! expressions, the optional `apply_nonblocking` setup line, the
//! emitted file-header + run.sh + relay-banner provenance text, and the
//! compiler-time `ContractGap` message prefix. Everything else (host
//! election, xfer registry, slice-paste tile derivation, accumulator
//! classification, FIFO-shape hazards) is identical and lives here.
//!
//! # No new dependency
//!
//! This module uses only `nucleus-compiler` contract types + other
//! `backend-common` modules (`multi_worker_walker`, `project_skeleton`,
//! `check_frame`, `render`, `host_election`). The `wire::...` strings
//! are EMITTED text that resolves in the GENERATED crate, not calls
//! here — so the lift introduces no `mp-tcp-common` dependency and no
//! backend dependency (a reverse arrow would cycle).

mod encode;
mod events;
mod plan;
mod relay;
pub mod walkers;
mod wire_primitives;
mod worker_program;

pub use encode::{decode_expr, encode_expr, scalar_fn_suffix, scalar_width};
pub use plan::{Plan, XferId};
pub use walkers::{
    collect_barriers_by_tag, collect_w2w_pushes, collect_xfer_data,
    detect_wait_before_push_hazard, relay_phase_insertion_point, RelayHop,
};
pub use wire_primitives::WirePrimitives;
