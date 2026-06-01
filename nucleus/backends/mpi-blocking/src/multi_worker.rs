//! Multi-worker SPMD codegen shim for the `mpi-blocking` backend
//! (TASK-0045.01; lifted to the shared substrate TASK-0046.02).
//!
//! The behaviour-bearing SPMD multi-worker `Plan` — host election, MPI
//! rank assignment, channel-id collection, barrier participant analysis
//! (whole-world `MPI_Barrier` vs strict-subset `MPI_Comm_split` sub-comm
//! barrier, TASK-0045.02), the multi-worker-check-frame loud reject, the
//! single-producer/single-consumer-per-pair guard, the shared
//! `render_worker_events` walk (`rendezvous_prefix = "mpi"`), pre-init,
//! and accumulator classification — lives ONCE in
//! [`backend_common::mpi_plan`], parameterised over the
//! [`backend_common::mpi_plan::MpiRendezvous`] trait (TASK-0046.02
//! lift). This file supplies only mpi-blocking's `MpiRendezvous` impl —
//! the BLOCKING `MPI_Send`/`MPI_Recv` rendezvous prelude (+ the
//! shared `WorldBar`/`SubcommBar` barrier wrappers) + the plain
//! (no buffer attach) `Universe` init — plus a `Plan` type alias and the
//! `render_main_rs_multi` entry the crate's `lib.rs` calls.
//!
//! mpi-nonblocking is the sibling consumer; its shim
//! (`nucleus/backends/mpi-nonblocking/src/multi_worker.rs`) supplies the
//! buffered-`Ibsend` variant + the `MPI_Bsend` buffer attach. The two
//! impls are the COMPLETE, enumerable delta between the backends —
//! everything else is shared.
//!
//! # Lowering map (realized by the shared substrate)
//!
//! - `Push{data, dst, seq}` -> `mpi_<rid>.push(<data>.clone())` lowering
//!   to a blocking standard-mode `MPI_Send` with **MPI tag = `<rid>`**
//!   (the per-rid tag is load-bearing for value-correctness; see
//!   [`backend_common::mpi_plan`]).
//! - `Wait{data, src, seq}` -> `let <data> = mpi_<rid>.wait();` lowering
//!   to a count-probing blocking `MPI_Recv`, same tag = `<rid>`.
//! - `Sync{tag}` -> `bar_<tag>.wait()`: a whole-world `MPI_Barrier` for a
//!   barrier whose participants are all used workers, or an
//!   `MPI_Comm_split` sub-communicator barrier for a strict-subset (e.g.
//!   host-excluding) participant set (TASK-0045.02; see
//!   [`backend_common::mpi_plan`]).
//!
//! # Standard-mode `MPI_Send` limitation
//!
//! `MPI_Send` (standard mode) may block for messages above the eager
//! limit until the matching `Recv` is posted (see the crate-root
//! limitation note). Documented, not a codegen reject; the buffered
//! non-blocking alternative is the M8 mpi-nonblocking backend.

use std::fmt::Write as _;

use nucleus_compiler::event::{Event, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;

use crate::NameTables;
use backend_common::mpi_plan::MpiRendezvous;
use backend_common::render::EmitError;

/// mpi-blocking's concrete `Plan` — the shared SPMD multi-worker
/// substrate with the blocking rendezvous variation bound in. `lib.rs`
/// reaches the substrate only through [`render_main_rs_multi`] below.
type Plan<'a> = backend_common::mpi_plan::Plan<'a, BlockingRendezvous>;

/// mpi-blocking rendezvous: blocking `MPI_Send` / `MPI_Recv`. The prelude
/// is EMITTED text resolving against the `mpi` crate in the GENERATED
/// project (not a dependency of this backend).
struct BlockingRendezvous;

impl MpiRendezvous for BlockingRendezvous {
    const BACKEND_NAME: &'static str = "mpi-blocking";

    fn write_header_dispatch_lines(out: &mut String, n: usize) {
        writeln!(
            out,
            "//! `world.rank()`. Push/Wait => blocking MPI Send/Recv (tag = rendezvous id);"
        )
        .ok();
        writeln!(
            out,
            "//! Sync => MPI_Barrier (whole-world, or Comm_split sub-comm for a strict subset)."
        )
        .ok();
        writeln!(out, "//! Launch: `mpiexec -n {n}`.").ok();
    }

    fn prelude() -> &'static str {
        // Blocking rendezvous + barrier wrapper types. `dead_code`: a
        // given schedule uses VecChan (array transfers) and/or ScalarChan
        // (scalar transfers); the unused one is dead in that project.
        "\
use mpi::traits::*;
// `Color` is referenced only by the emitted Comm_split calls (absent in
// a whole-world-only schedule); allow the unused import there.
#[allow(unused_imports)]
use mpi::topology::{Color, Process, SimpleCommunicator};
use mpi::{Rank, Tag};
use core::marker::PhantomData;

/// One whole-array (`Vec<T>`) cross-worker rendezvous over MPI. `push`
/// is a blocking standard-mode `MPI_Send` of the slice; `wait` is a
/// blocking `MPI_Recv` (count-probing `receive_vec`). Both pin the MPI
/// TAG to the rendezvous id so concurrent transfers between the same
/// rank pair cannot cross-match (see backend module docstring).
//
// `dead_code`: a given schedule uses VecChan (array transfers) and/or
// ScalarChan (scalar transfers); the unused one is dead in that project.
#[allow(dead_code)]
struct VecChan<'a, T: Equivalence> { proc_: Process<'a>, tag: Tag, _pd: PhantomData<T> }
#[allow(dead_code)]
impl<'a, T: Equivalence> VecChan<'a, T> {
    fn new<C: Communicator>(comm: &'a C, peer: Rank, tag: Tag) -> Self {
        VecChan { proc_: comm.process_at_rank(peer), tag, _pd: PhantomData }
    }
    fn push(&self, v: Vec<T>) { self.proc_.send_with_tag(&v[..], self.tag); }
    fn wait(&self) -> Vec<T> { self.proc_.receive_vec_with_tag::<T>(self.tag).0 }
}

/// Scalar (`T`) cross-worker rendezvous over MPI. Same tag discipline as
/// [`VecChan`]; sends/receives a single element.
#[allow(dead_code)]
struct ScalarChan<'a, T: Equivalence> { proc_: Process<'a>, tag: Tag, _pd: PhantomData<T> }
#[allow(dead_code)]
impl<'a, T: Equivalence> ScalarChan<'a, T> {
    fn new<C: Communicator>(comm: &'a C, peer: Rank, tag: Tag) -> Self {
        ScalarChan { proc_: comm.process_at_rank(peer), tag, _pd: PhantomData }
    }
    fn push(&self, v: T) { self.proc_.send_with_tag(&v, self.tag); }
    fn wait(&self) -> T { self.proc_.receive_with_tag::<T>(self.tag).0 }
}

/// Whole-world barrier wrapper. `wait` is `MPI_Barrier` over COMM_WORLD;
/// every rank participates.
#[allow(dead_code)]
struct WorldBar<'a, C: Communicator> { comm: &'a C }
#[allow(dead_code)]
impl<'a, C: Communicator> WorldBar<'a, C> {
    fn new(comm: &'a C) -> Self { WorldBar { comm } }
    fn wait(&self) { self.comm.barrier(); }
}

/// Sub-communicator barrier wrapper for a strict-subset (e.g.
/// host-excluding) barrier. Holds the `Option<SimpleCommunicator>` that
/// `world.split_by_color(..)` returned for THIS rank — `Some(subcomm)`
/// if the rank is a participant (it passed `Color::with_value`), `None`
/// if it is not (it passed `Color::undefined()`). `wait` is
/// `MPI_Barrier` over the SUB-communicator, called ONLY by participants;
/// a non-participant's `None` makes `wait` a no-op. The split itself is
/// the collective (every rank calls it, OUTSIDE the rank arms); `wait`
/// is collective only over the participant sub-group, so a no-op on the
/// excluded ranks is correct, not a skipped collective. `dead_code`: a
/// whole-world-only schedule emits no `SubcommBar`.
#[allow(dead_code)]
struct SubcommBar<'a> { sub: &'a Option<SimpleCommunicator> }
#[allow(dead_code)]
impl<'a> SubcommBar<'a> {
    fn new(sub: &'a Option<SimpleCommunicator>) -> Self { SubcommBar { sub } }
    fn wait(&self) { if let Some(c) = self.sub { c.barrier(); } }
}
"
    }

    fn write_universe_init(out: &mut String, _bsend_bytes: usize) {
        // Plain `Universe` init — blocking send needs no buffer attach,
        // so the buffer-size heuristic is ignored here.
        writeln!(
            out,
            "    // MPI_Init. The `Universe` guard runs MPI_Finalize on drop on EVERY rank."
        )
        .ok();
        writeln!(
            out,
            "    let universe = mpi::initialize().expect(\"MPI_Init failed\");"
        )
        .ok();
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
