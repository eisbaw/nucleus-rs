//! MPI sub-communicator barrier smoke binary (TASK-0045.02 AC#3).
//!
//! SPMD: one binary, every rank runs `main`, behaviour dispatches on
//! `MPI_Comm_rank` — the exact shape the two MPI backends' shared
//! substrate (`backend_common::mpi_plan`) generates for a STRICT-SUBSET
//! (e.g. host-excluding) barrier. It exercises the runtime primitives
//! the substrate lowers a non-whole-world `Event::Sync` onto:
//!
//!   * `world.split_by_color(Color::with_value(c))`  (participant)   ─┐
//!   * `world.split_by_color(Color::undefined())`    (non-participant)┘
//!         -> MPI_Comm_split, COLLECTIVE over COMM_WORLD (EVERY rank
//!            calls it, in identical order, OUTSIDE any per-rank arm —
//!            the AC#2 deadlock-safety invariant)
//!   * `subcomm.barrier()`                            (participant only)
//!         -> MPI_Barrier over the participant SUB-group (the
//!            non-participants hold `None` and never call it)
//!
//! Layout (rank 0 == the elected "host", excluded; ranks 1.. are the
//! "compute" participants — exactly the 09-producer-consumer shape that
//! motivates this lowering):
//!
//!   rank 0 (host)        : NON-participant. Splits with Color::undefined
//!                          (-> None), takes part in NO sub-comm barrier.
//!   ranks 1..size (compute): participants. Split with the shared color,
//!                          land in one sub-communicator, and barrier on
//!                          it. The lowest participant broadcasts a
//!                          sentinel THROUGH the sub-communicator and a
//!                          sub-comm barrier orders it; every participant
//!                          re-checks the value so a broken split / wrong
//!                          sub-group fails the smoke LOUD.
//!
//! The whole program must run to completion under `mpiexec -n N` (N>=3,
//! so the host-excluding subset is a STRICT subset of >=2 participants)
//! WITHOUT deadlocking: if the host wrongly joined the participant
//! sub-group (or a participant skipped the collective split), the
//! sub-comm barrier would block forever and `just
//! check-mpi-barrier-smoke`'s `timeout` would fail LOUD.

use mpi::topology::Color;
use mpi::traits::*;

/// Shared color of the compute sub-group (any non-negative value; the
/// generated code uses the 0-based distinct-subset index, here 0).
const COMPUTE_COLOR: std::os::raw::c_int = 0;

/// Sentinel the lowest participant broadcasts through the sub-comm — an
/// arbitrary fixed value every participant re-checks so a wrong sub-group
/// / corrupted collective fails the smoke.
const SENTINEL: i64 = 0x5EED_BA77;

fn main() {
    let universe = mpi::initialize().expect("MPI_Init failed");
    let world = universe.world();
    let size = world.size();
    let rank = world.rank();

    println!("rank {rank} of {size}");

    // Need at least one host (rank 0) + two compute participants so the
    // participant set is a STRICT subset of >= 2 ranks (the case a plain
    // world.barrier() could not serve). Guard so a stray small launch is
    // a clear message, not a hang.
    if size < 3 {
        eprintln!(
            "barrier-smoke: need >=3 ranks (1 excluded host + >=2 compute participants), got {size}"
        );
        std::process::exit(2);
    }

    // --- COLLECTIVE split over COMM_WORLD: EVERY rank calls it, with the
    //     same call shape, in the same place. rank 0 is the excluded host
    //     (Color::undefined -> None); ranks 1.. are compute participants
    //     (shared color -> Some(subcomm)). This mirrors the generated
    //     `let split_0 = world.split_by_color(if [..].contains(&rank) {..}
    //     else { Color::undefined() });` emitted OUTSIDE the rank arms.
    let is_participant = rank != 0;
    let subcomm = world.split_by_color(if is_participant {
        Color::with_value(COMPUTE_COLOR)
    } else {
        Color::undefined()
    });

    match subcomm {
        // --- Compute participants: barrier on the SUB-communicator. ---
        Some(sub) => {
            let sub_rank = sub.rank();
            let sub_size = sub.size();
            assert_eq!(
                sub_size,
                size - 1,
                "the compute sub-group must hold every rank except the excluded host \
                 (expected {} participants, sub-comm has {sub_size})",
                size - 1
            );

            // Sub-comm-ordered broadcast: the lowest participant
            // (sub-rank 0) sets the sentinel; a sub-comm barrier orders it
            // before the others read their copy. (A whole-WORLD barrier
            // here would deadlock — the host never reaches this code.)
            let mut value: i64 = if sub_rank == 0 { SENTINEL } else { 0 };
            sub.process_at_rank(0)
                .broadcast_into(&mut value);
            sub.barrier(); // <- the SubcommBar.wait() the backend emits

            assert_eq!(
                value, SENTINEL,
                "barrier-smoke: sub-rank {sub_rank} saw {value:#x}, expected {SENTINEL:#x} \
                 (Comm_split / sub-comm broadcast+barrier corrupted the value)"
            );
            println!("compute participant world-rank {rank} (sub-rank {sub_rank}/{sub_size}) barrier OK");
        }
        // --- Excluded host: holds None, does NOT touch the sub-comm. ---
        None => {
            assert_eq!(rank, 0, "only the excluded host (rank 0) gets Color::undefined");
            println!("host world-rank {rank} excluded from the compute barrier (no sub-comm) OK");
        }
    }
}
