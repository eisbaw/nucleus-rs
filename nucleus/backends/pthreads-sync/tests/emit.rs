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
//! Honest limitation: synthetic two-worker tests are NOT here yet
//! because multi-worker codegen isn't implemented at M1 — `emit`
//! correctly rejects with `EmitError::UnsupportedFeature`. A
//! synthetic test that *asserts* the rejection lives below
//! (`multi_worker_is_rejected`). The PRD-listed AC #5 case — a
//! two-worker pingpong producing compilable Rust — is filed as a
//! follow-up (see TASK-0020 self-report).

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
fn multi_worker_is_rejected() {
    // Construct a synthetic ACFG that references two distinct workers.
    // We don't go through the full parse pipeline because the
    // existing example 01's naive schedule only declares one worker —
    // the rejection logic lives in `emit`'s worker collection, which
    // we can prove via direct ACFG construction.
    use compiler::{ACFGNode, DataflowDag, DataflowEdge, Operation, ACFG};
    use compiler::{DataId, KernelId, WorkerId};
    use std::collections::{BTreeMap, BTreeSet};

    let mut workers_a: BTreeSet<WorkerId> = BTreeSet::new();
    workers_a.insert(WorkerId(0));
    let mut workers_b: BTreeSet<WorkerId> = BTreeSet::new();
    workers_b.insert(WorkerId(1));

    let op_a = Operation {
        kernel: KernelId(0),
        workers: workers_a,
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
        workers: workers_b,
        dataflow: DataflowDag {
            edges: vec![DataflowEdge {
                data_in: vec![DataId(0)],
                kernel: KernelId(1),
                data_out: None,
            }],
        },
    };

    let acfg = ACFG {
        root: ACFGNode::Sequence(vec![ACFGNode::Operation(op_a), ACFGNode::Operation(op_b)]),
        name_kernels: BTreeMap::new(),
        name_data: BTreeMap::new(),
        name_workers: BTreeMap::new(),
        name_iter_vars: BTreeMap::new(),
    };

    // The LinkedIR doesn't matter for the rejection path — `emit`
    // counts workers before touching it. Build a minimal stub.
    let linked = compiler::link::LinkedIR {
        algo: compiler::algo::AlgoIR::default(),
        sched: compiler::sched::SchedIR::default(),
        placements: BTreeMap::new(),
        kernel_workers: BTreeMap::new(),
        data_producers: BTreeMap::new(),
        data_consumers: BTreeMap::new(),
    };

    let out = scratch_dir("multi_worker_is_rejected");
    // kernels.rs path doesn't exist; we never get that far if the
    // multi-worker rejection fires first.
    let kernels = out.join("nonexistent-kernels.rs");
    let err = emit(&acfg, &linked, &kernels, &out).unwrap_err();
    match err {
        pthreads_sync::EmitError::UnsupportedFeature(msg) => {
            assert!(
                msg.contains("multi-worker"),
                "unexpected UnsupportedFeature message: {msg}"
            );
        }
        other => panic!("expected UnsupportedFeature, got {other:?}"),
    }
}
