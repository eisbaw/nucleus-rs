//! Multi-worker SPMD codegen shim for the `mpi-blocking` backend
//! (TASK-0045.01; lifted to the shared substrate TASK-0046.02).
//!
//! The behaviour-bearing SPMD multi-worker `Plan` — host election, MPI
//! rank assignment, channel-id collection, barrier participant
//! analysis, the non-whole-world-barrier + multi-worker-check-frame loud
//! rejects, the single-producer/single-consumer-per-pair guard, the
//! shared `render_worker_events` walk (`rendezvous_prefix = "mpi"`),
//! pre-init, and accumulator classification — lives ONCE in
//! [`backend_common::mpi_plan`], parameterised over the
//! [`backend_common::mpi_plan::MpiRendezvous`] trait (TASK-0046.02
//! lift). This file supplies only mpi-blocking's `MpiRendezvous` impl —
//! the BLOCKING `MPI_Send`/`MPI_Recv` rendezvous prelude + the plain
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
//! - `Sync{tag}` -> `bar_<tag>.wait()` == a whole-world `MPI_Barrier`.
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
        writeln!(out, "//! Sync => whole-world MPI_Barrier. Launch: `mpiexec -n {n}`.").ok();
    }

    fn prelude() -> &'static str {
        // Blocking rendezvous + barrier wrapper types. `dead_code`: a
        // given schedule uses VecChan (array transfers) and/or ScalarChan
        // (scalar transfers); the unused one is dead in that project.
        "\
use mpi::traits::*;
use mpi::topology::Process;
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
/// every rank participates (the emit rejects non-whole-world barriers).
struct WorldBar<'a, C: Communicator> { comm: &'a C }
impl<'a, C: Communicator> WorldBar<'a, C> {
    fn new(comm: &'a C) -> Self { WorldBar { comm } }
    fn wait(&self) { self.comm.barrier(); }
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
