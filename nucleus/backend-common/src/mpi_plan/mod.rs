//! Shared multi-worker SPMD emit substrate for the tier-2 MPI backends
//! (mpi-blocking, mpi-nonblocking). TASK-0046.02.
//!
//! # Why this module exists
//!
//! `mpi-nonblocking` was created (TASK-0046, M8) by copying
//! `mpi-blocking`'s `src/multi_worker.rs` `Plan` logic VERBATIM and
//! swapping the rendezvous prelude (blocking `MPI_Send`/`MPI_Recv` →
//! buffered `MPI_Ibsend` + `MPI_Mprobe`/`MPI_Imrecv`/`MPI_Wait`). That
//! verbatim copy is a silent-sibling hazard: a future fix to the
//! host-election / rank-assignment / barrier-analysis / loud-reject
//! logic could land in one backend and silently skip the other. Two
//! consumers proved the abstraction well-shaped, so this module lifts
//! the behaviour-bearing `Plan` into a single source of truth
//! parameterised over the [`MpiRendezvous`] trait — the same "fail
//! fast, prove a second consumer, then lift" precedent as the sync-TCP
//! lift ([`super::tcp_plan`], TASK-0044.02.03), the event lift
//! ([`super::event_plan`], TASK-0044.03.02) and the original
//! backend-common lift (TASK-0244).
//!
//! Each backend crate now reduces to a thin shim: an [`MpiRendezvous`]
//! impl plus a `type Plan<'a> = backend_common::mpi_plan::Plan<'a,
//! ThisRendezvous>` alias and a `render_main_rs_multi` wrapper.
//!
//! # Distinct from the multi-binary TCP/event substrates
//!
//! The sync-TCP and event backends emit N SEPARATE binaries wired over
//! sockets. MPI is structurally different — **SPMD**: ONE binary, every
//! rank runs `main`, behaviour branches on `world.rank()` (PRD §7.2).
//! So this is a SEPARATE substrate; it does not share `Plan` internals
//! with [`super::tcp_plan`] / [`super::event_plan`], only the
//! trait-over-`Plan` shape.
//!
//! # Variation surface
//!
//! [`MpiRendezvous`] is the SOLE axis of variation between the two MPI
//! backends. See its docstring for the complete enumeration; in short:
//! the backend name in compiler-time diagnostics, the emitted
//! file-header dispatch-banner prose, the emitted rendezvous +
//! barrier wrapper-type prelude, and the emitted `Universe` init block
//! (the blocking backend takes `world` directly; the buffered backend
//! attaches an `MPI_Bsend` buffer first). Everything else — host
//! election, rank assignment, channel-id collection, barrier
//! participant analysis (whole-world `MPI_Barrier` vs strict-subset
//! `MPI_Comm_split` sub-comm barrier, TASK-0045.02), the
//! multi-worker-check-frame loud reject, the
//! single-producer/single-consumer-per-pair guard, the
//! `render_worker_events` walk (`rendezvous_prefix = "mpi"`), pre-init,
//! accumulator classification — is identical and lives here.
//!
//! # No new dependency
//!
//! This module uses only `nucleus-compiler` contract types + other
//! `backend-common` modules (`multi_worker_walker`, `host_election`,
//! `render`). The `mpi::...` strings are EMITTED text that resolves in
//! the GENERATED crate (which depends on rsmpi), NOT calls here — so
//! the lift introduces no `mpi`/`rsmpi` dependency and no backend
//! dependency (a reverse arrow would cycle).

mod plan;
mod rendezvous;

pub use plan::Plan;
pub use rendezvous::MpiRendezvous;
