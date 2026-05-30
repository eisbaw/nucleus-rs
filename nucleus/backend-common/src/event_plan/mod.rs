//! Shared multi-process emit substrate for the async event-reactor
//! backends (mp-tcp-event, mp-uds-event). TASK-0044.03.02.
//!
//! # Why this module exists
//!
//! `mp-uds-event` was created (cycle 197, TASK-0044.03.01) by copying
//! `mp-tcp-event`'s `src/multi_worker/` subtree (~3000 LoC) verbatim
//! and swapping the transport (TCP loopback → Unix domain sockets). Two
//! consumers proved the abstraction well-shaped, so this module lifts
//! the behaviour-bearing plan / walkers / encode / relay /
//! worker-program logic into a single source of truth parameterised
//! over the [`EventTransport`] trait — the same "fail fast, prove a
//! second consumer, then lift" precedent as the original backend-common
//! lift (TASK-0244) and the sibling sync-TCP lift ([`super::tcp_plan`],
//! TASK-0044.02.03).
//!
//! Each backend crate now reduces to a thin shim: an `EventTransport`
//! impl plus a `type Plan<'a> = backend_common::event_plan::Plan<'a,
//! ThisTransport>` alias and a `render_run_sh` wrapper, with re-exports
//! of the shared `Plan` so the crate's `lib.rs` and in-crate tests
//! resolve unchanged.
//!
//! # Distinct from the sync-TCP substrate
//!
//! This is a SEPARATE substrate from [`super::tcp_plan`]: the sync-TCP
//! backends emit a blocking/poll FIFO `Plan` (one ordered DATA stream
//! per pair, host-relay reads in event order), whereas the event
//! backends emit a mio-reactor `Plan` with per-`(seq, peer)` bounded
//! outbound queues + per-seq inbound demux (host-relay forwards via
//! `Reactor::relay_one`). The two lifts share the trait-over-Plan
//! shape but NOT the Plan internals.
//!
//! # Variation surface
//!
//! [`EventTransport`] is the SOLE axis of variation. See its docstring
//! for the complete enumeration; in short: the emitted file-header
//! line, the CTRL/DATA stream-type spellings, the role-specific
//! imports, the rendezvous handshake block, the `run.sh`
//! post-processing + SO_BUF commentary, and the compiler-time
//! `ContractGap` message prefix. Everything else (host election, chan
//! registry, capacity lookup, peer-index routing, barrier analysis,
//! host-relay, the per-worker event walk, accumulator classification,
//! FIFO-shape hazards) is identical and lives here.
//!
//! # No new dependency
//!
//! This module uses only `nucleus-compiler` contract types + other
//! `backend-common` modules (`multi_worker_walker`, `project_skeleton`,
//! `check_frame`, `render`, `host_election`). The transport / wire
//! strings are EMITTED text that resolves in the GENERATED crate, not
//! calls here — so the lift introduces no `mp-*-event` dependency and
//! no backend dependency (a reverse arrow would cycle). `mio` is
//! referenced only as emitted text; `backend-common` does not depend on
//! it.

mod encode;
mod plan;
mod relay;
mod transport;
pub(crate) mod walkers;
mod worker_program;

// TASK-0044.03.02.01: only the Plan API is re-exported `pub` for the backend
// shims. The walker/encode helpers are crate-internal substrate (reached by
// the sibling event_plan submodules via the `walkers::`/`encode::` module
// paths directly) and the `walkers` module is `pub(crate)`, so no substrate
// internals leak across the backend-common crate boundary. The earlier
// top-level walker/encode re-exports here were a dead external surface (no
// in-crate consumer) — removed rather than kept as unused `pub(crate)`.
pub use plan::{ChanId, Plan};
pub use transport::EventTransport;
