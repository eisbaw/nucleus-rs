//! Multi-worker SPMD codegen shim for the `mpi-nonblocking` backend
//! (TASK-0046, M8; lifted to the shared substrate TASK-0046.02).
//!
//! The behaviour-bearing SPMD multi-worker `Plan` — host election, MPI
//! rank assignment, channel-id collection, barrier participant
//! analysis, the non-whole-world-barrier + multi-worker-check-frame loud
//! rejects, the single-producer/single-consumer-per-pair guard, the
//! shared `render_worker_events` walk (`rendezvous_prefix = "mpi"`),
//! pre-init, accumulator classification, AND the buffered-send buffer
//! size heuristic ([`backend_common::mpi_plan::Plan::bsend_bytes`]) —
//! lives ONCE in [`backend_common::mpi_plan`], parameterised over the
//! [`backend_common::mpi_plan::MpiRendezvous`] trait (TASK-0046.02
//! lift). This file supplies only mpi-nonblocking's `MpiRendezvous`
//! impl — the NON-BLOCKING BUFFERED `MPI_Ibsend` + `MPI_Mprobe`/
//! `MPI_Imrecv`/`MPI_Wait` rendezvous prelude + the `Universe` init that
//! attaches the `MPI_Bsend` buffer — plus a `Plan` type alias and the
//! `render_main_rs_multi` entry the crate's `lib.rs` calls.
//!
//! mpi-blocking is the sibling consumer; its shim
//! (`nucleus/backends/mpi-blocking/src/multi_worker.rs`) supplies the
//! blocking `Send`/`Recv` variant + the plain (no buffer attach)
//! `Universe` init. The two impls are the COMPLETE, enumerable delta
//! between the backends — everything else is shared.
//!
//! # Lowering map (realized by the shared substrate)
//!
//! - `Push{data, dst, seq}` -> `mpi_<rid>.push(<data>.clone())` lowering
//!   to a buffered `MPI_Ibsend` that completes LOCALLY (the payload is
//!   copied into the process-attached send buffer), so the push never
//!   blocks on the matching receive, with **MPI tag = `<rid>`** (the
//!   per-rid tag is load-bearing for value-correctness; see
//!   [`backend_common::mpi_plan`]).
//! - `Wait{data, src, seq}` -> `let <data> = mpi_<rid>.wait();` lowering
//!   to `MPI_Mprobe` + `MPI_Imrecv` + `MPI_Wait` (whole-array) or
//!   `MPI_Irecv` (scalar), same tag = `<rid>`.
//! - `Sync{tag}` -> `bar_<tag>.wait()` == a whole-world `MPI_Barrier`.
//!
//! # Why buffered (not standard) non-blocking send (the deadlock fix)
//!
//! The async schedules order their `Push`es BEFORE the matching `Wait`s
//! (a `Sync` may sit between), the way the fully-async pthreads `Slot`
//! model permits. Under blocking `MPI_Send` (mpi-blocking) a message
//! above the eager limit blocks until its receive is posted, so that
//! ordering can DEADLOCK. Standard-mode `MPI_Isend` would avoid the
//! block but its request borrows the send buffer until `Wait`, forcing
//! the `Wait` to be deferred past the matching receive for any overlap —
//! which the linear `.push(v)` walker API cannot express without a
//! self-referential buffer/request or unbounded leaking. Buffered
//! `MPI_Ibsend` sidesteps both: it copies the payload into the attached
//! buffer and completes locally, so `push` can `Wait` immediately (clean
//! buffer lifetime) yet never blocks on the receiver. The cost is the
//! attached buffer (sized by the shared
//! [`backend_common::mpi_plan::Plan::bsend_bytes`] heuristic, attached
//! in the `Universe` init below); the benefit is genuine
//! deadlock-immunity.

use std::fmt::Write as _;

use nucleus_compiler::event::{Event, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;

use crate::NameTables;
use backend_common::mpi_plan::MpiRendezvous;
use backend_common::render::EmitError;

/// mpi-nonblocking's concrete `Plan` — the shared SPMD multi-worker
/// substrate with the buffered rendezvous variation bound in. `lib.rs`
/// reaches the substrate only through [`render_main_rs_multi`] below.
type Plan<'a> = backend_common::mpi_plan::Plan<'a, BufferedRendezvous>;

/// mpi-nonblocking rendezvous: buffered `MPI_Ibsend` + `MPI_Mprobe`/
/// `MPI_Imrecv`/`MPI_Wait`. The prelude is EMITTED text resolving against
/// the `mpi` crate in the GENERATED project (not a dependency of this
/// backend).
struct BufferedRendezvous;

impl MpiRendezvous for BufferedRendezvous {
    const BACKEND_NAME: &'static str = "mpi-nonblocking";

    fn write_header_dispatch_lines(out: &mut String, n: usize) {
        writeln!(
            out,
            "//! `world.rank()`. Push => buffered MPI_Ibsend (tag = rendezvous id, local"
        )
        .ok();
        writeln!(
            out,
            "//! completion); Wait => MPI_Imrecv/Irecv + MPI_Wait; Sync => whole-world"
        )
        .ok();
        writeln!(out, "//! MPI_Barrier. Launch: `mpiexec -n {n}`.").ok();
    }

    fn prelude() -> &'static str {
        // Buffered (non-blocking, local-completion) rendezvous + barrier
        // wrapper types. `dead_code`: a given schedule uses VecChan (array
        // transfers) and/or ScalarChan (scalar transfers); the unused one
        // is dead in that project.
        "\
use mpi::traits::*;
use mpi::topology::Process;
use mpi::{Rank, Tag};
use core::marker::PhantomData;

/// One whole-array (`Vec<T>`) cross-worker rendezvous over MPI,
/// NON-BLOCKING BUFFERED. `push` is `MPI_Ibsend` + immediate `MPI_Wait`
/// inside a `mpi::request::scope`: buffered mode copies the payload into
/// the process-attached send buffer (`MPI_Buffer_attach`, set in `main`)
/// and the Wait completes LOCALLY — it does NOT block on the matching
/// receive, which is what makes a wait-before-push / cyclic worker<->worker
/// exchange deadlock-immune above the MPI eager limit. `wait` is
/// `MPI_Mprobe` (to learn the element count) + `MPI_Imrecv` + `MPI_Wait`.
/// Both pin the MPI TAG to the rendezvous id so concurrent transfers
/// between the same rank pair cannot cross-match (see backend module
/// docstring).
///
/// Buffer-lifetime safety: `v` / `buf` is owned here and the request
/// borrows it only inside the scope closure, which runs the Wait before
/// the buffer drops; rsmpi's `Request` panics on drop if left uncompleted
/// (a runtime backstop against a forgotten Wait).
//
// `dead_code`: a given schedule uses VecChan (array transfers) and/or
// ScalarChan (scalar transfers); the unused one is dead in that project.
#[allow(dead_code)]
struct VecChan<'a, T: Equivalence> { proc_: Process<'a>, tag: Tag, _pd: PhantomData<T> }
#[allow(dead_code)]
impl<'a, T: Equivalence + Default + Clone> VecChan<'a, T> {
    fn new<C: Communicator>(comm: &'a C, peer: Rank, tag: Tag) -> Self {
        VecChan { proc_: comm.process_at_rank(peer), tag, _pd: PhantomData }
    }
    fn push(&self, v: Vec<T>) {
        // MPI_Ibsend (buffered, local completion) + MPI_Wait. The scope
        // closure completes the request before `v` drops.
        mpi::request::scope(|scope| {
            self.proc_
                .immediate_buffered_send_with_tag(scope, &v[..], self.tag)
                .wait_without_status();
        });
    }
    fn wait(&self) -> Vec<T> {
        // MPI_Mprobe sizes the buffer; MPI_Imrecv + MPI_Wait fills it. The
        // matched probe guarantees the subsequent receive takes THIS
        // message (no same-tag race).
        let (msg, status) = self.proc_.matched_probe_with_tag(self.tag);
        let count = status.count(T::equivalent_datatype()) as usize;
        let mut buf: Vec<T> = vec![T::default(); count];
        mpi::request::scope(|scope| {
            msg.immediate_matched_receive_into(scope, &mut buf[..])
                .wait_without_status();
        });
        buf
    }
}

/// Scalar (`T`) cross-worker rendezvous over MPI, NON-BLOCKING BUFFERED.
/// Same tag + buffered-`Ibsend` discipline as [`VecChan`]; the receive is
/// `MPI_Irecv` of a single element (`ReceiveFuture`) + its `Wait`.
#[allow(dead_code)]
struct ScalarChan<'a, T: Equivalence> { proc_: Process<'a>, tag: Tag, _pd: PhantomData<T> }
#[allow(dead_code)]
impl<'a, T: Equivalence> ScalarChan<'a, T> {
    fn new<C: Communicator>(comm: &'a C, peer: Rank, tag: Tag) -> Self {
        ScalarChan { proc_: comm.process_at_rank(peer), tag, _pd: PhantomData }
    }
    fn push(&self, v: T) {
        mpi::request::scope(|scope| {
            self.proc_
                .immediate_buffered_send_with_tag(scope, &v, self.tag)
                .wait_without_status();
        });
    }
    fn wait(&self) -> T {
        // MPI_Irecv of one element + Wait (ReceiveFuture::get).
        self.proc_.immediate_receive_with_tag::<T>(self.tag).get().0
    }
}

/// Whole-world barrier wrapper. `wait` is `MPI_Barrier` over COMM_WORLD;
/// every rank participates (the emit rejects non-whole-world barriers).
struct WorldBar<'a, C: Communicator> { comm: &'a C }
impl<'a, C: Communicator> WorldBar<'a, C> {
    fn new(comm: &'a C) -> Self { WorldBar { comm } }
    fn wait(&self) { self.comm.barrier(); }
}
"
    }

    fn write_universe_init(out: &mut String, bsend_bytes: usize) {
        // Buffered-send buffer attach (MPI_Buffer_attach). Buffered
        // `Ibsend` copies each payload here and completes locally, so a
        // Push never blocks on the matching Wait (the deadlock fix). Sized
        // from the schedule's data footprint with headroom
        // (`bsend_bytes`, the shared Plan heuristic);
        // `NUC_MPI_BSEND_BYTES` overrides for an unusually large in-flight
        // set. A too-small buffer fails LOUD (MPI_ERR_BUFFER), never
        // silent corruption. Attached BEFORE `world` is taken (the attach
        // needs `&mut universe`).
        writeln!(
            out,
            "    // MPI_Init. The `Universe` guard runs MPI_Finalize on drop on EVERY rank."
        )
        .ok();
        writeln!(
            out,
            "    let mut universe = mpi::initialize().expect(\"MPI_Init failed\");"
        )
        .ok();
        writeln!(
            out,
            "    // MPI_Buffer_attach: buffered Ibsend copies payloads here (local completion)."
        )
        .ok();
        writeln!(
            out,
            "    let bsend_bytes: usize = std::env::var(\"NUC_MPI_BSEND_BYTES\")"
        )
        .ok();
        writeln!(
            out,
            "        .ok().and_then(|s| s.parse::<usize>().ok()).unwrap_or({bsend_bytes})"
        )
        .ok();
        writeln!(
            out,
            "        // MPI_Buffer_attach size is a C int; cap so an oversized override fails"
        )
        .ok();
        writeln!(
            out,
            "        // loud (MPI_ERR_BUFFER) rather than panicking in the C-int conversion."
        )
        .ok();
        writeln!(out, "        .min(i32::MAX as usize);").ok();
        writeln!(out, "    universe.set_buffer_size(bsend_bytes);").ok();
        writeln!(out, "    let world = universe.world();").ok();
    }
}

/// Render the full `main.rs` of the multi-worker SPMD MPI program.
pub(crate) fn render_main_rs_multi(
    per_worker: &std::collections::BTreeMap<WorkerId, Vec<Event>>,
    names: &NameTables,
    sidecar: &NameSidecar,
) -> Result<String, EmitError> {
    Plan::build(per_worker, names, sidecar)?.emit()
}
