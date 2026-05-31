//! Single-worker SPMD emit pins + the multi-worker forward-link guard
//! for mpi-blocking (TASK-0045). In-crate (`#[cfg(test)] mod tests;`) so
//! it can pin the private compute-body delegation against the shared
//! renderer directly.

use std::collections::BTreeMap;
use std::path::PathBuf;

use nucleus_compiler::event::{Event, WorkerId};

use crate::{emit, EmitError};

fn repo_root() -> PathBuf {
    // .../nucleus/backends/mpi-blocking -> backends -> nucleus -> repo.
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(std::path::Path::parent)
        .and_then(std::path::Path::parent)
        .expect("three ancestors above mpi-blocking crate")
        .to_path_buf()
}

/// Lower 01-elementwise-add/naive — the smallest real single-worker
/// witness (a kernel call + sidecar consumption, not an empty scaffold).
fn lower_01() -> test_common::LowerForTestResult {
    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples/01-elementwise-add");
    let algo_src = std::fs::read_to_string(ex.join("prog.algo.nuc")).expect("01 algo");
    let sched_src =
        std::fs::read_to_string(ex.join("schedules/naive.sched.nuc")).expect("01 sched");
    test_common::lower_for_test(
        &algo_src,
        &sched_src,
        &test_common::LowerForTestOpts {
            apply_block_transforms: false,
            apply_partition_workers: false,
            inject_check_frames: false,
        },
    )
}

#[test]
fn single_worker_emit_shape_and_compute_delegation() {
    let r = lower_01();
    let ex = repo_root().join("nuc-nucleus/examples/01-elementwise-add");
    let kernels = ex.join("kernels.rs");

    let scratch = repo_root().join("nucleus/target/mpi-blocking-test-scratch/single_worker_01");
    let _ = std::fs::remove_dir_all(&scratch);

    let res = emit(&r.per_worker, &r.names, &r.sidecar, &kernels, &scratch)
        .expect("mpi-blocking emit (single-worker 01-elementwise-add)");

    // --- compute.rs is EXACTLY the shared single-worker renderer output
    //     (no mpi-specific wrapping/mangling of the arithmetic). Drift
    //     detector: any wrapper around the delegated emitter diffs here.
    let used: Vec<WorkerId> = r
        .per_worker
        .iter()
        .filter(|(_, e)| !e.is_empty())
        .map(|(w, _)| *w)
        .collect();
    assert_eq!(used.len(), 1, "01/naive must lower to a single used worker");
    let events: &[Event] = r.per_worker.get(&used[0]).map(Vec::as_slice).unwrap();
    let expected_compute = pthreads_sync::render_single_worker_main_with_signature(
        events,
        &r.names,
        &r.sidecar,
        crate::KERNELS_MOD_ATTR,
        crate::COMPUTE_FN_SIGNATURE,
    )
    .expect("shared renderer");
    let compute = std::fs::read_to_string(&res.compute_rs).expect("compute.rs");
    assert_eq!(
        compute, expected_compute,
        "compute.rs MUST be byte-identical to the shared single-worker \
         renderer output (delegation, not re-implementation)"
    );

    // Non-vacuous witness: the compute body actually calls the kernel.
    assert!(
        compute.contains("kernels::add"),
        "01-elementwise-add witness must emit kernels::add (else vacuous):\n{compute}"
    );

    // --- main.rs is the SPMD MPI wrapper.
    let main_rs = std::fs::read_to_string(&res.main_rs).expect("main.rs");
    for needle in [
        "mpi::initialize()",
        "world.rank() == 0",
        "compute::nuc_compute()",
        "use mpi::traits::Communicator as _;",
    ] {
        assert!(
            main_rs.contains(needle),
            "SPMD main.rs must contain `{needle}`:\n{main_rs}"
        );
    }

    // --- Cargo.toml pulls in rsmpi; run.sh launches via mpiexec.
    let cargo = std::fs::read_to_string(&res.cargo_toml).expect("Cargo.toml");
    assert!(
        cargo.contains("mpi = \"0.8\"") && cargo.contains("nuc-generated"),
        "Cargo.toml must depend on rsmpi (`mpi = \"0.8\"`) and build nuc-generated:\n{cargo}"
    );
    let run_sh = std::fs::read_to_string(&res.run_sh).expect("run.sh");
    assert!(
        run_sh.contains("mpiexec") && run_sh.contains("NUC_MPI_RANKS"),
        "run.sh must launch via mpiexec with a configurable rank count:\n{run_sh}"
    );

    // kernels.rs is a verbatim copy of the source.
    let emitted_kernels = std::fs::read_to_string(&res.kernels_rs).expect("kernels.rs");
    let src_kernels = std::fs::read_to_string(&kernels).expect("source kernels.rs");
    assert_eq!(
        emitted_kernels, src_kernels,
        "kernels.rs must be a verbatim copy of the source"
    );
}

#[test]
fn multi_worker_is_a_loud_forward_link_not_a_silent_emit() {
    // Build a synthetic 2-used-worker map by replaying the real
    // single-worker event list under two distinct WorkerIds. This
    // exercises the `used_workers.len() > 1` guard without depending on
    // partition-lowering details.
    let r = lower_01();
    let used: Vec<WorkerId> = r
        .per_worker
        .iter()
        .filter(|(_, e)| !e.is_empty())
        .map(|(w, _)| *w)
        .collect();
    let evs = r.per_worker.get(&used[0]).cloned().expect("01 events");

    let mut two: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    two.insert(WorkerId(0), evs.clone());
    two.insert(WorkerId(1), evs);

    let kernels = repo_root().join("nuc-nucleus/examples/01-elementwise-add/kernels.rs");
    let scratch = repo_root().join("nucleus/target/mpi-blocking-test-scratch/multi_worker_reject");
    let _ = std::fs::remove_dir_all(&scratch);

    let err = emit(&two, &r.names, &r.sidecar, &kernels, &scratch)
        .expect_err("multi-worker mpi-blocking must be rejected this cycle");
    match err {
        EmitError::ContractGap(msg) => {
            assert!(
                msg.contains("TASK-0045.01") && msg.contains("multi-worker"),
                "the rejection must forward-link the multi-worker arm (TASK-0045.01):\n{msg}"
            );
        }
        other => panic!("expected ContractGap forward-link, got {other:?}"),
    }
}
