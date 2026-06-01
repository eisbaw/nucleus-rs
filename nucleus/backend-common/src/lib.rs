//! Shared codegen primitives consumed by every tier-1 backend
//! (pthreads-sync, pthreads-async, mp-tcp-bufsync). TASK-0244.
//!
//! # Why this crate exists
//!
//! Through cycles 17/22/24/31 the shared codegen surface grew inside
//! `pthreads-sync` — it was historically the first backend, so the
//! shared code lived there and the other backends imported it via
//! `pthreads_sync::*` paths. This made pthreads-async and
//! mp-tcp-bufsync look like dependents of pthreads-sync, which is
//! NOT semantically true: all three are siblings, each emits a
//! different runtime substrate (Slot vs Ring vs TCP), and none is the
//! parent of the others. The arrow was leaking the implementation
//! detail "pthreads-sync was extracted first" as if it were structure.
//!
//! TASK-0244 (cycle 37) moves the shared code into THIS crate. Every
//! backend now depends on `backend-common`; none depends on another
//! backend except where the dependency is genuinely semantic —
//! pthreads-async's single-worker arm DELEGATES to
//! `pthreads_sync::render_single_worker_main` to guarantee byte-
//! identical single-worker emit, and that delegation is the one
//! remaining inter-backend arrow.
//!
//! # Module map
//!
//! - [`check_frame`] — `check loop V : on_violation={panic,log,count}`
//!   emit templates and the `CountCheckLoop` materialised view.
//! - [`render`] — `RenderCtx` + the expression / index / kernel-call /
//!   loop-bound / type renderers shared across single- and multi-
//!   worker emit on every backend. Also houses [`EmitError`] (the
//!   codegen-time error type re-exported by every backend's public
//!   surface).
//! - [`multi_worker_walker`] — the per-worker `Event` walker shared
//!   between pthreads-sync's `multi_worker::Plan::emit`, pthreads-
//!   async's `multi_worker::Plan::emit`, and (future) any backend
//!   whose multi-worker substrate is rendezvous-channel-shaped.
//! - [`project_skeleton`] — Cargo.toml + run.sh string templates for
//!   the single-binary tier-1 backends (pthreads-sync + pthreads-async).
//!   TASK-0246 (cycle 38): finishes the cycle-37 extraction by lifting
//!   the last two non-semantic shared strings out of pthreads-sync.
//!   mp-tcp-bufsync's multi-process variant stays in mp-tcp-bufsync
//!   (different signature, different shape).
//! - [`host_election`] — the canonical host-election rule shared by
//!   every tier-1 backend's `multi_worker::Plan::build` AND by the
//!   compiler-level passes in `nucleus-driver` that mediate against
//!   the backend-elected host (cycles 160 / 162 / 163). TASK-0336
//!   cycle 164 lift; retires the
//!   `feedback-driver-must-mirror-backend-election-exactly`
//!   recurrence surface on the canonical path.
//! - [`tcp_plan`] — the shared multi-process emit `Plan` substrate for
//!   the sync-TCP backends (mp-tcp-bufsync, mp-tcp-poll), parameterised
//!   over the `WirePrimitives` trait (blocking vs nonblocking-poll wire
//!   primitives). TASK-0044.02.03 lift: the ~1.4k LoC of plan / walkers
//!   / encode logic that the two sync-TCP backends had duplicated
//!   verbatim now lives here; each backend supplies only a
//!   `WirePrimitives` impl + a `Plan` type alias.
//! - [`event_plan`] — the shared multi-process emit `Plan` substrate
//!   for the async event-reactor backends (mp-tcp-event, mp-uds-event),
//!   parameterised over the `EventTransport` trait (TCP loopback vs
//!   Unix domain sockets). TASK-0044.03.02 lift: the ~3k LoC of mio
//!   reactor Plan / walkers / encode / relay / worker-program logic the
//!   two event backends had duplicated verbatim now lives here; each
//!   backend supplies only an `EventTransport` impl + a `Plan` type
//!   alias. A SEPARATE substrate from `tcp_plan` (per-seq-demux reactor
//!   vs FIFO).
//! - [`mpi_plan`] — the shared SPMD multi-worker emit `Plan` substrate
//!   for the tier-2 MPI backends (mpi-blocking, mpi-nonblocking),
//!   parameterised over the `MpiRendezvous` trait (blocking Send/Recv
//!   vs buffered Ibsend/Imrecv prelude + buffer attach). TASK-0046.02
//!   lift: the multi-worker `Plan` (host election, rank assignment,
//!   channel-id collection, barrier analysis, the loud rejects, the
//!   `render_worker_events` walk) that mpi-nonblocking had duplicated
//!   verbatim from mpi-blocking now lives here; each backend supplies
//!   only an `MpiRendezvous` impl + a `Plan` type alias. SPMD (one
//!   rank-dispatched binary) — distinct from the multi-binary
//!   `tcp_plan` / `event_plan` substrates.

pub mod check_frame;
pub mod event_plan;
pub mod host_election;
pub mod mpi_plan;
pub mod multi_worker_walker;
pub mod project_skeleton;
pub mod render;
pub mod tcp_plan;

// Convenience top-level re-exports of the codegen surface that consumers
// actually reach through the crate root, so backends can write
// `backend_common::EmitError` instead of `backend_common::render::EmitError`.
//
// Re-derived TASK-0411 (workspace consumer grep, comment lines filtered):
// only THREE re-exported names have crate-root-path code consumers today —
// `EmitError` (every backend further re-exports it from its own public
// surface; 11 root consumers), `elect_host_from_name_workers` (6), and
// `elect_host_from_worker_names` (5). The other ~32 names that used to be
// re-exported here had ZERO root-path consumers — every real consumer
// reaches them via the *submodule* path (`backend_common::render::X`,
// `backend_common::check_frame::X`, …) — so the root re-exports were dead
// weight on this internal (unpublished) crate and were removed. The
// `pub mod` declarations above ARE the submodule paths consumers use;
// in-crate intra-doc links (e.g. [`render::RenderCtx`]) resolve via the
// defining module, not via these root re-exports, so removal is
// doc-link-safe (verified by a `cargo doc --no-deps` before/after diff —
// the gate does not build docs; see feedback-visibility-tighten-doclink-trap).
// `EmitError`'s crate-root intra-doc link at the top of this file stays
// because `EmitError` stays.
pub use host_election::{elect_host_from_name_workers, elect_host_from_worker_names};
pub use render::EmitError;
