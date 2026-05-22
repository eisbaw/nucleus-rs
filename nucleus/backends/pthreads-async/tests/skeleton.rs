//! Skeleton + single-worker smoke tests for the pthreads-async backend.
//!
//! TASK-0042.01 cycle 17 (2026-05-22): the single-worker arm
//! (TASK-0226) is implemented; multi-worker (TASK-0228) is still a
//! typed ContractGap. These tests pin both edges:
//!
//! 1. Multi-worker `emit()` rejects with `EmitError::ContractGap`
//!    whose message names the backend AND forward-links TASK-0228 —
//!    the precise next-step pointer the next implementer hits when
//!    they wire up the ring-buffer + Condvar arm.
//!
//! 2. Single-worker `emit()` succeeds for an empty event-list (the
//!    smallest legal input) AND produces a `main.rs` byte-identical
//!    to `pthreads_sync::emit`'s on the same input — the
//!    cross-backend differential invariant on naive schedules holds
//!    by construction (same delegated renderer).
//!
//! 3. The `EmitResult` shape is the single-binary five-field shape
//!    (mirrors pthreads-sync). Compile-time only: if a later cycle
//!    changes the struct, this `let _r = EmitResult { ... }` won't
//!    compile and the driver dispatch arm has to be updated in lockstep.

use std::collections::BTreeMap;
use std::path::PathBuf;

use compiler::event::{Event, WorkerId};
use compiler::sidecar::NameSidecar;
use pthreads_async::{emit, EmitError, EmitResult, NameTables};

/// Build a 2-worker `per_worker` map of empty event lists. Useful for
/// hitting the multi-worker dispatch arm without actually emitting any
/// Event variants. WorkerId values are constructed via the public
/// constructor (compiler::event::WorkerId is `pub`).
fn two_workers_empty() -> BTreeMap<WorkerId, Vec<Event>> {
    let mut m: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    // Empty event lists trip the used_workers filter (it requires
    // non-empty). Add ONE no-op Event::Alloc per worker so they count
    // as "used" — Alloc is the safest variant because the single-worker
    // emitter explicitly ignores it (pthreads-sync lib.rs:1030).
    use compiler::event::{DataId, IterTile, Region};
    let a = Event::Alloc {
        data: DataId(0),
        tile: IterTile::default(),
        region: Region(0),
    };
    let b = Event::Alloc {
        data: DataId(1),
        tile: IterTile::default(),
        region: Region(0),
    };
    m.insert(WorkerId(0), vec![a]);
    m.insert(WorkerId(1), vec![b]);
    m
}

#[test]
fn multi_worker_emit_returns_contract_gap_with_task_0228_forward_link() {
    let per_worker = two_workers_empty();
    let names = NameTables::default();
    let sidecar = NameSidecar::default();
    // Bogus paths — multi-worker arm short-circuits before any I/O.
    let kernels = PathBuf::from("/does-not-exist/kernels.rs");
    let out = PathBuf::from("/does-not-exist/out");

    let res = emit(&per_worker, &names, &sidecar, &kernels, &out);

    match res {
        Err(EmitError::ContractGap(msg)) => {
            assert!(
                msg.contains("pthreads-async"),
                "multi-worker ContractGap message must name the backend: {msg}"
            );
            assert!(
                msg.contains("TASK-0228"),
                "multi-worker ContractGap message must forward-link TASK-0228 \
                 (the ring-buffer/Condvar/Plan headline work): {msg}"
            );
            assert!(
                msg.contains("multi-worker") || msg.contains("ring buffer"),
                "multi-worker ContractGap message must declare its scope: {msg}"
            );
        }
        Err(other) => panic!(
            "expected EmitError::ContractGap from multi-worker arm, got: {other:?}"
        ),
        Ok(result) => panic!(
            "multi-worker emit() must Err; got Ok({result:?}). Did TASK-0228 \
             land without removing this test? Delete it as part of that cycle."
        ),
    }
}

#[test]
fn single_worker_empty_eventlist_emits_byte_identical_to_pthreads_sync() {
    // Empty input: ZERO used workers. Both backends should emit
    // identical Cargo.toml + main.rs + run.sh. The temp dirs are
    // distinct but the FILE CONTENTS must match byte-for-byte —
    // that's the cross-backend differential invariant on the empty
    // schedule (the smallest possible witness).
    let per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    let names = NameTables::default();
    let sidecar = NameSidecar::default();

    // Use the manifest-dir + a unique subdir under target/ so the test
    // works under cargo's per-test sandbox without TMPDIR assumptions.
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent() // backends/
        .and_then(|p| p.parent()) // nucleus/
        .map(|p| p.join("target"))
        .expect("workspace target/");
    let stem = target.join("pthreads-async-test-scratch/single_worker_empty");
    let async_out = stem.join("async");
    let sync_out = stem.join("sync");
    // Clean any prior run so write_file is deterministic.
    let _ = std::fs::remove_dir_all(&async_out);
    let _ = std::fs::remove_dir_all(&sync_out);

    // Both backends accept a `kernels.rs` that exists but is empty —
    // empty event list -> no kernel call site -> kernels.rs content
    // does not appear in main.rs. We use the same file for both
    // backends so the kernels.rs copy step in each is identical.
    let kernels = stem.join("empty_kernels.rs");
    std::fs::create_dir_all(&stem).expect("scratch dir");
    std::fs::write(&kernels, "// Empty kernels.rs for the empty-eventlist test.\n")
        .expect("empty kernels.rs");

    let async_res =
        emit(&per_worker, &names, &sidecar, &kernels, &async_out).expect("async emit");
    // Build pthreads_sync's NameTables from the re-exported alias.
    let sync_res = pthreads_sync::emit(&per_worker, &names, &sidecar, &kernels, &sync_out)
        .expect("sync emit");

    // ---- The invariant: byte-identical main.rs, Cargo.toml, run.sh ----
    let async_main = std::fs::read_to_string(&async_res.main_rs).expect("async main.rs");
    let sync_main = std::fs::read_to_string(&sync_res.main_rs).expect("sync main.rs");
    assert_eq!(
        async_main, sync_main,
        "pthreads-async single-worker main.rs must be byte-identical \
         to pthreads-sync's (the cross-backend differential invariant): \n\
         async:\n{async_main}\n--- sync:\n{sync_main}"
    );

    let async_cargo =
        std::fs::read_to_string(&async_res.cargo_toml).expect("async Cargo.toml");
    let sync_cargo =
        std::fs::read_to_string(&sync_res.cargo_toml).expect("sync Cargo.toml");
    assert_eq!(
        async_cargo, sync_cargo,
        "pthreads-async single-worker Cargo.toml must be byte-identical \
         to pthreads-sync's (shared render_cargo_toml)"
    );

    let async_run = std::fs::read_to_string(&async_res.run_sh).expect("async run.sh");
    let sync_run = std::fs::read_to_string(&sync_res.run_sh).expect("sync run.sh");
    assert_eq!(
        async_run, sync_run,
        "pthreads-async single-worker run.sh must be byte-identical \
         to pthreads-sync's (shared render_run_sh)"
    );

    let async_kernels =
        std::fs::read_to_string(&async_res.kernels_rs).expect("async kernels.rs");
    let sync_kernels =
        std::fs::read_to_string(&sync_res.kernels_rs).expect("sync kernels.rs");
    assert_eq!(
        async_kernels, sync_kernels,
        "pthreads-async single-worker kernels.rs must be a verbatim copy \
         (same input file, same copy)"
    );
}

/// Locate the repo root from this crate's manifest. Mirrors the
/// pattern in pthreads-sync/tests/emit.rs:79 — the test must work
/// regardless of where cargo is invoked.
fn repo_root() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(std::path::Path::parent)
        .and_then(std::path::Path::parent)
        .expect("three ancestors above pthreads-async crate")
        .to_path_buf()
}

/// Build (per_worker, NameTables, NameSidecar, kernels.rs path) for
/// example 01-elementwise-add / naive.sched.nuc by running the full
/// compiler pipeline — same shape as pthreads-sync/tests/emit.rs's
/// `link_example_01_naive` + `contract_inputs`. Re-implemented here
/// rather than imported because cross-crate test helpers add a
/// non-trivial compile-graph dependency and the pattern is small.
fn lower_example_01_naive() -> (
    BTreeMap<WorkerId, Vec<Event>>,
    NameTables,
    NameSidecar,
    PathBuf,
) {
    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples/01-elementwise-add");
    let algo_src = std::fs::read_to_string(ex.join("prog.algo.nuc")).expect("01 algo");
    let sched_src =
        std::fs::read_to_string(ex.join("schedules/naive.sched.nuc")).expect("01 sched");

    // TASK-0237 (cycle 24): pipeline boilerplate moved to test_common.
    // 01-elementwise-add/naive is single-worker so no partition_workers
    // or inject_check_frames. Cycle-24 review-gate A.2: explicitly set
    // apply_block_transforms=false to restore the pre-cycle-24
    // byte-faithful behaviour (the original helper at cycle 17 did
    // NOT run block_transforms; on naive it's a no-op so neither
    // value matters at runtime, but the explicit false documents the
    // historical contract for any future schedule that would NOT be
    // a no-op).
    let r = test_common::lower_for_test(
        &algo_src,
        &sched_src,
        &test_common::LowerForTestOpts {
            apply_block_transforms: false,
            apply_partition_workers: false,
            inject_check_frames: false,
        },
    );
    let names = NameTables {
        data: r.name_data,
        kernel: r.name_kernel,
        worker: r.name_worker,
        iter_var: r.name_iter_var,
        inner_block_iter_vars: r.inner_block_iter_vars,
    };
    (r.per_worker, names, r.sidecar, ex.join("kernels.rs"))
}

/// Non-empty witness for the byte-identical-by-construction claim
/// (HIGH finding A1 from the cycle-17 review-gate). The empty-eventlist
/// test above proves the *scaffold* (mod header, fn main, no body)
/// matches; this test proves the *actual codegen* (kernel calls,
/// loop headers, sidecar-driven pre-init, real symbolic loops) also
/// matches.
///
/// If a future drift adds a pthreads-async-specific wrapper around
/// the delegated `render_single_worker_main` (e.g. an async-runtime
/// prelude), this test fails: the prelude appears in the async
/// output but not the sync output. That is exactly the drift the
/// commit-message "byte-identical by construction" wording is
/// promising.
///
/// Example 01-elementwise-add / naive is the smallest non-trivial
/// witness — one input, one output, one kernel call inside one
/// loop. Real loop, real Fire, real sidecar consumption.
#[test]
fn single_worker_real_example_emits_byte_identical_to_pthreads_sync() {
    let (per_worker, names, sidecar, kernels) = lower_example_01_naive();

    let scratch = repo_root().join("nucleus/target/pthreads-async-test-scratch/single_worker_01_naive");
    let async_out = scratch.join("async");
    let sync_out = scratch.join("sync");
    let _ = std::fs::remove_dir_all(&async_out);
    let _ = std::fs::remove_dir_all(&sync_out);

    let async_res = emit(&per_worker, &names, &sidecar, &kernels, &async_out)
        .expect("pthreads-async emit (single-worker real example)");
    let sync_res = pthreads_sync::emit(&per_worker, &names, &sidecar, &kernels, &sync_out)
        .expect("pthreads-sync emit (same input)");

    // The cross-backend differential invariant: same algorithm + same
    // naive schedule -> byte-identical main.rs across both backends.
    let async_main =
        std::fs::read_to_string(&async_res.main_rs).expect("async main.rs");
    let sync_main = std::fs::read_to_string(&sync_res.main_rs).expect("sync main.rs");
    assert_eq!(
        async_main, sync_main,
        "pthreads-async single-worker main.rs on 01-elementwise-add/naive \
         MUST be byte-identical to pthreads-sync's (the cross-backend \
         differential invariant). A diff here means the delegation to \
         pthreads_sync::render_single_worker_main was bypassed or wrapped:\n\
         === async main.rs ===\n{async_main}\n\
         === sync main.rs ===\n{sync_main}"
    );

    // Sanity: the witness IS non-trivial. The emitted main.rs must
    // contain a kernel call (so we are not vacuously identical because
    // both emitters output nothing). 01-elementwise-add's kernel is
    // `add`.
    assert!(
        async_main.contains("kernels::add"),
        "non-trivial witness check: 01-elementwise-add emits a kernels::add \
         call; absence of it would mean the test passed vacuously:\n{async_main}"
    );
}

#[test]
fn emit_result_shape_is_single_binary_five_fields() {
    // Compile-time pin: if the EmitResult struct gains/loses a field
    // this test stops compiling. The driver dispatch arm
    // (`nucleus/driver/src/main.rs`, the `"pthreads-async"` match arm)
    // must update in lockstep — pinning the shape here keeps the two
    // in sync. **Kept beyond TASK-0226** (per its AC#3): the
    // ContractGap-message test is gone but the struct shape pin lives
    // as long as `EmitResult` does.
    let _r = EmitResult {
        project_dir: PathBuf::from("/p"),
        cargo_toml: PathBuf::from("/p/Cargo.toml"),
        main_rs: PathBuf::from("/p/src/main.rs"),
        kernels_rs: PathBuf::from("/p/src/kernels.rs"),
        run_sh: PathBuf::from("/p/run.sh"),
    };
    let _ = _r;
}
