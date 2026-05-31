//! Single-worker SPMD emit pins + the multi-worker forward-link guard
//! for mpi-blocking (TASK-0045). In-crate (`#[cfg(test)] mod tests;`) so
//! it can pin the private compute-body delegation against the shared
//! renderer directly.

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
    let compute_rs_path = res
        .compute_rs
        .as_ref()
        .expect("single-worker arm emits a separate compute.rs");
    let compute = std::fs::read_to_string(compute_rs_path).expect("compute.rs");
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

/// Lower 02-split-add/split — the canonical 2-worker (host + w0)
/// multi-worker witness: host runs the I/O kernels, w0 runs the `add`
/// loop; three `sync` transfers cross (a, b host->w0; c w0->host); three
/// whole-world `{host,w0}` barriers.
fn lower_02_split() -> test_common::LowerForTestResult {
    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples/02-split-add");
    let algo_src = std::fs::read_to_string(ex.join("prog.algo.nuc")).expect("02 algo");
    let sched_src =
        std::fs::read_to_string(ex.join("schedules/split.sched.nuc")).expect("02 split sched");
    test_common::lower_for_test(
        &algo_src,
        &sched_src,
        &test_common::LowerForTestOpts {
            apply_block_transforms: false,
            apply_partition_workers: true,
            inject_check_frames: false,
        },
    )
}

#[test]
fn multi_worker_02_split_spmd_shape_and_tag_discipline() {
    let r = lower_02_split();
    let used: Vec<WorkerId> = r
        .per_worker
        .iter()
        .filter(|(_, e)| !e.is_empty())
        .map(|(w, _)| *w)
        .collect();
    assert_eq!(used.len(), 2, "02-split/split must lower to two used workers");

    let kernels = repo_root().join("nuc-nucleus/examples/02-split-add/kernels.rs");
    let scratch = repo_root().join("nucleus/target/mpi-blocking-test-scratch/multi_worker_02");
    let _ = std::fs::remove_dir_all(&scratch);

    let res = emit(&r.per_worker, &r.names, &r.sidecar, &kernels, &scratch)
        .expect("mpi-blocking multi-worker emit (02-split/split)");

    // No separate compute.rs in the multi-worker arm — the whole
    // rank-dispatched program lives in main.rs.
    assert!(
        res.compute_rs.is_none(),
        "multi-worker arm must not emit a separate compute.rs"
    );

    let main_rs = std::fs::read_to_string(&res.main_rs).expect("main.rs");

    // SPMD dispatch + MPI lifecycle + rendezvous prelude + barrier.
    for needle in [
        "mpi::initialize()",
        "let world = universe.world();",
        "match world.rank() {",
        "if size != 2 {",
        "struct VecChan",
        "send_with_tag",
        "receive_vec_with_tag",
        "struct WorldBar",
        ".barrier();",
        "mod kernels;",
    ] {
        assert!(
            main_rs.contains(needle),
            "multi-worker main.rs must contain `{needle}`:\n{main_rs}"
        );
    }

    // Two rank arms (host = rank 0, w0 = rank 1).
    assert!(
        main_rs.contains("0 => {") && main_rs.contains("1 => {"),
        "main.rs must dispatch rank 0 (host) and rank 1 (w0):\n{main_rs}"
    );

    // The arithmetic still calls the user kernel (non-vacuous witness).
    assert!(
        main_rs.contains("kernels::add"),
        "the w0 arm must call kernels::add:\n{main_rs}"
    );

    // Tag discipline (LOAD-BEARING): each channel binding pins the MPI
    // tag to the rendezvous id — the `::new(&world, <peer>, <rid>)` third
    // argument equals the `mpi_<rid>` index. Without per-rid tags,
    // same-rank-pair messages cross-match by send order. Verify every
    // emitted binding self-consistently uses its own rid as the tag.
    let mut checked = 0;
    for line in main_rs.lines() {
        if let Some(pos) = line.find("let mpi_") {
            let after = &line[pos + "let mpi_".len()..];
            let rid: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            assert!(!rid.is_empty(), "malformed chan binding: {line}");
            // The `new(&world, <peer>, <tag>)` call's last arg must be rid.
            let args_open = line.find("::new(&world,").expect("chan binding shape");
            let args = &line[args_open..];
            let last_arg = args
                .trim_end_matches(';')
                .rsplit(',')
                .next()
                .expect("tag arg")
                .trim()
                .trim_start_matches(|c: char| !c.is_ascii_digit())
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>();
            assert_eq!(
                last_arg, rid,
                "chan mpi_{rid} must use tag == rid {rid} (got tag `{last_arg}`):\n{line}"
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 3,
        "02-split has 3 cross-worker transfers => >=3 chan bindings (saw {checked}):\n{main_rs}"
    );
}

#[test]
fn non_whole_world_barrier_is_rejected_loud() {
    // Mutate a real lowering: drop the host from one barrier's
    // participant set so it becomes host-excluding. The emit must reject
    // with a typed UnsupportedFeature forward-linked to the Comm_split
    // follow-up — NOT emit an untested sub-communicator collective.
    let mut r = lower_02_split();
    let host = {
        // host election mirrors the backend: the worker named "host".
        r.names
            .worker
            .iter()
            .find(|(_, n)| n.as_str() == "host")
            .map(|(w, _)| *w)
            .expect("02-split has a `host` worker")
    };
    let mut mutated = false;
    for evs in r.per_worker.values_mut() {
        for e in evs.iter_mut() {
            if let Event::Sync { participants, .. } = e {
                if participants.remove(&host) {
                    mutated = true;
                }
            }
        }
    }
    assert!(mutated, "expected at least one Sync to drop the host");

    let kernels = repo_root().join("nuc-nucleus/examples/02-split-add/kernels.rs");
    let scratch = repo_root().join("nucleus/target/mpi-blocking-test-scratch/non_whole_world_bar");
    let _ = std::fs::remove_dir_all(&scratch);

    let err = emit(&r.per_worker, &r.names, &r.sidecar, &kernels, &scratch)
        .expect_err("a host-excluding barrier must be rejected (Comm_split unproven)");
    match err {
        EmitError::UnsupportedFeature(msg) => assert!(
            msg.contains("Comm_split") && msg.contains("TASK-0045.02"),
            "rejection must forward-link the Comm_split follow-up:\n{msg}"
        ),
        other => panic!("expected UnsupportedFeature(Comm_split), got {other:?}"),
    }
}
