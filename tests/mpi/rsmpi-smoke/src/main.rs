//! rsmpi smoke binary. See ../Cargo.toml for why this exists.
//!
//! SPMD: one binary, every rank runs `main`, behaviour dispatches on
//! `MPI_Comm_rank` — the exact shape the `mpi-blocking` backend
//! generates (PRD §7.2). Exercises the runtime primitives the backend
//! lowers an EventList onto:
//!
//!   * `mpi::initialize()` / drop          -> MPI_Init / MPI_Finalize
//!   * `world.size()` / `world.rank()`     -> MPI_Comm_size / _rank
//!   * `world.process_at_rank(d).send(..)` -> blocking MPI_Send  (Push)
//!   * `world.any_process().receive()`     -> blocking MPI_Recv  (Wait)
//!
//! Rank 0 sends a sentinel to rank 1, rank 1 checks it. Any mismatch or
//! transport error aborts non-zero so `just check-mpi-smoke` fails LOUD.

use mpi::traits::*;

/// Sentinel rank 0 sends to rank 1 — an arbitrary fixed value the
/// receiver re-checks so a silently-wrong Send/Recv fails the smoke.
const SENTINEL: i64 = 0x5EED_F00D;

fn main() {
    let universe = mpi::initialize().expect("MPI_Init failed");
    let world = universe.world();
    let size = world.size();
    let rank = world.rank();

    println!("rank {rank} of {size}");

    // Need at least two ranks to exercise point-to-point. The smoke is
    // always launched with -n >= 2; guard anyway so a stray -n 1 run is
    // a clear message, not a hang.
    if size < 2 {
        eprintln!("rsmpi-smoke: need >=2 ranks to exercise Send/Recv (got {size})");
        std::process::exit(2);
    }

    if rank == 0 {
        world.process_at_rank(1).send(&SENTINEL);
    } else if rank == 1 {
        let (got, _status) = world.any_process().receive::<i64>();
        assert_eq!(
            got, SENTINEL,
            "rsmpi-smoke: rank 1 received {got:#x}, expected {SENTINEL:#x} \
             (blocking Send/Recv corrupted the payload)"
        );
        println!("rank 1 received sentinel OK");
    }
    // Ranks >= 2 (under an -n > 2 launch) just report presence — the
    // backend's barrier/relay lowering is what fans out to them; the
    // smoke only needs to prove one Send/Recv hop links and runs.
}
