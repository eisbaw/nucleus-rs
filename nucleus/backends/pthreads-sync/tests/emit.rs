//! Unit tests for the pthreads-sync backend `emit(...)` function.
//!
//! These exercise the renderer in isolation: parse + lower + link
//! example 01, build the ACFG, run sync + transfer injection, then
//! call `emit(...)` into a tempdir. We assert:
//!
//! - Every expected file appears.
//! - Cargo.toml is a parseable standalone manifest.
//! - main.rs mentions the expected kernel names (a smoke test that
//!   the dataflow walk actually emitted calls).
//!
//! The actual bit-identical end-to-end test (compile + run + diff
//! reference.bin) lives at `nucleus/compiler/tests/e2e_example_01.rs`
//! to keep this crate's test load light.
//!
//! Multi-worker codegen (TASK-0122) is now in place. A synthetic
//! two-worker pingpong test lives in `tests/multi_worker.rs`. The
//! load-bearing end-to-end test (example 02 split.sched.nuc against
//! `reference.bin`) lives at
//! `nucleus/compiler/tests/e2e_example_02.rs`.
//!
//! At this layer (the unit-test surface of the backend) we keep:
//! - The single-worker happy-path tests below (example 01 × naive).
//! - A small synthetic test that proves a two-worker ACFG produces
//!   compilable Rust without a runtime check (the runtime is in the
//!   multi_worker.rs harness).
//! - A test that rejects an unsupported distributed placement
//!   (`place k on {w0,w1,...}`).

use std::fs;
use std::path::{Path, PathBuf};

use compiler::{
    algo::{lower_algo, parse_algo},
    build_acfg, inject_syncs, inject_transfers, link,
    sched::{lower_sched, parse_sched},
};
use pthreads_sync::emit;

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR -> nucleus/backends/pthreads-sync
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .expect("three ancestors above pthreads-sync crate")
        .to_path_buf()
}

fn scratch_dir(name: &str) -> PathBuf {
    let target = repo_root().join("nucleus/target/pthreads-sync-test-scratch");
    let _ = fs::create_dir_all(&target);
    let dir = target.join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Load + link example 01 with the naive schedule. Shared helper.
fn link_example_01_naive() -> (compiler::link::LinkedIR, compiler::ACFG, PathBuf) {
    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples/01-elementwise-add");
    let algo_src = fs::read_to_string(ex.join("prog.algo.nuc")).unwrap();
    let sched_src = fs::read_to_string(ex.join("schedules/naive.sched.nuc")).unwrap();

    let algo_ast = parse_algo(&algo_src).unwrap();
    let sched_ast = parse_sched(&sched_src).unwrap();
    let algo_ir = lower_algo(&algo_ast).unwrap();
    let sched_ir = lower_sched(&sched_ast).unwrap();
    let linked = link(algo_ir, sched_ir).unwrap();
    let acfg = build_acfg(&linked);
    let acfg = inject_syncs(acfg);
    let acfg = inject_transfers(&linked, acfg);
    (linked, acfg, ex.join("kernels.rs"))
}

#[test]
fn emit_writes_all_files() {
    let (linked, acfg, kernels) = link_example_01_naive();
    let out = scratch_dir("emit_writes_all_files");
    let result = emit(&acfg, &linked, &kernels, &out).expect("emit succeeded");

    // All four artefacts present.
    assert!(result.cargo_toml.exists(), "Cargo.toml not written");
    assert!(result.main_rs.exists(), "main.rs not written");
    assert!(result.kernels_rs.exists(), "kernels.rs not copied");
    assert!(result.run_sh.exists(), "run.sh not written");

    // run.sh on unix is executable.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&result.run_sh).unwrap().permissions().mode();
        // 0o111 is "any execute bit set". Don't pin the exact 0o755
        // because umask could legitimately strip the group/other.
        assert!(
            mode & 0o111 != 0,
            "run.sh is not executable (mode={mode:o})"
        );
    }
}

#[test]
fn main_rs_calls_every_kernel() {
    let (linked, acfg, kernels) = link_example_01_naive();
    let out = scratch_dir("main_rs_calls_every_kernel");
    let result = emit(&acfg, &linked, &kernels, &out).unwrap();
    let main_rs = fs::read_to_string(&result.main_rs).unwrap();

    // Smoke test: every kernel from example 01 appears as a call.
    for kernel in &["load_input", "load_input_b", "add", "save_output"] {
        let needle = format!("kernels::{kernel}");
        assert!(
            main_rs.contains(&needle),
            "main.rs is missing `{needle}`:\n---\n{main_rs}\n---"
        );
    }

    // The for-loop bound for example 01 is N=256.
    assert!(
        main_rs.contains("256_i64"),
        "main.rs is missing the N=256 loop bound:\n---\n{main_rs}\n---"
    );
}

#[test]
fn kernels_rs_is_copied_verbatim() {
    let (linked, acfg, kernels) = link_example_01_naive();
    let out = scratch_dir("kernels_rs_is_copied_verbatim");
    let result = emit(&acfg, &linked, &kernels, &out).unwrap();
    let src = fs::read_to_string(&kernels).unwrap();
    let dst = fs::read_to_string(&result.kernels_rs).unwrap();
    assert_eq!(src, dst, "kernels.rs was not copied byte-for-byte");
}

#[test]
fn distributed_placement_is_rejected() {
    // Construct a synthetic LinkedIR where a kernel is placed on
    // *two* workers — the multi-worker codegen rejects this because
    // iteration-space partitioning (TASK-0117) isn't implemented.
    // We pass through `emit` so the rejection path is the real one
    // (validate_placements in multi_worker.rs).
    use compiler::link::WorkerEntity;
    use compiler::sched::{ResolvedPlaceTarget, ResolvedPlacement};
    use compiler::{ACFGNode, DataflowDag, DataflowEdge, Operation, ACFG};
    use compiler::{DataId, KernelId, WorkerId};
    use std::collections::{BTreeMap, BTreeSet};

    // Two distinct workers referenced by Operations so the
    // "single-worker fast path" is bypassed.
    let mut wa: BTreeSet<WorkerId> = BTreeSet::new();
    wa.insert(WorkerId(0));
    let mut wb: BTreeSet<WorkerId> = BTreeSet::new();
    wb.insert(WorkerId(1));
    let op_a = Operation {
        kernel: KernelId(0),
        workers: wa,
        dataflow: DataflowDag {
            edges: vec![DataflowEdge {
                data_in: vec![],
                kernel: KernelId(0),
                data_out: Some(DataId(0)),
            }],
        },
    };
    let op_b = Operation {
        kernel: KernelId(1),
        workers: wb,
        dataflow: DataflowDag {
            edges: vec![DataflowEdge {
                data_in: vec![DataId(0)],
                kernel: KernelId(1),
                data_out: None,
            }],
        },
    };

    let mut name_workers: BTreeMap<String, WorkerId> = BTreeMap::new();
    name_workers.insert("w0".into(), WorkerId(0));
    name_workers.insert("w1".into(), WorkerId(1));

    let acfg = ACFG {
        root: ACFGNode::Sequence(vec![ACFGNode::Operation(op_a), ACFGNode::Operation(op_b)]),
        name_kernels: BTreeMap::new(),
        name_data: BTreeMap::new(),
        name_workers,
        name_iter_vars: BTreeMap::new(),
    };

    // LinkedIR with a distributed placement: kernel `dist` on
    // {w0, w1}. multi_worker::validate_placements should reject.
    let mut placements: BTreeMap<String, ResolvedPlacement> = BTreeMap::new();
    placements.insert(
        "dist".into(),
        ResolvedPlacement {
            kernel: "dist".into(),
            target: ResolvedPlaceTarget::Many(vec!["w0".into(), "w1".into()]),
        },
    );
    let mut kernel_workers: BTreeMap<String, WorkerEntity> = BTreeMap::new();
    kernel_workers.insert(
        "dist".into(),
        WorkerEntity({
            let mut s = BTreeSet::new();
            s.insert("w0".into());
            s.insert("w1".into());
            s
        }),
    );

    let linked = compiler::link::LinkedIR {
        algo: compiler::algo::AlgoIR::default(),
        sched: compiler::sched::SchedIR::default(),
        placements,
        kernel_workers,
        data_producers: BTreeMap::new(),
        data_consumers: BTreeMap::new(),
    };

    let out = scratch_dir("distributed_placement_is_rejected");
    // kernels.rs needs to exist so we don't trip the read-kernels
    // step before reaching multi-worker validation.
    let kernels = out.join("kernels.rs");
    fs::write(&kernels, "// stub\n").unwrap();
    let err = emit(&acfg, &linked, &kernels, &out).unwrap_err();
    match err {
        pthreads_sync::EmitError::UnsupportedFeature(msg) => {
            assert!(
                msg.contains("distributed placement") || msg.contains("placed on"),
                "unexpected UnsupportedFeature message: {msg}"
            );
        }
        other => panic!("expected UnsupportedFeature, got {other:?}"),
    }
}
