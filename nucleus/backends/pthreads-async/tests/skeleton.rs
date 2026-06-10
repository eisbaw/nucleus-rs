//! Skeleton + single-worker smoke tests for the pthreads-async backend.
//!
//! Cycle history:
//! - **TASK-0226 (cycle 17):** single-worker arm implemented (delegates
//!   to `backend_common::single_worker_main::render_single_worker_main`). The byte-identical-
//!   to-pthreads-sync invariant on naive schedules holds by
//!   construction (same delegated renderer).
//! - **TASK-0228 Wave B-2 (cycle 26):** multi-worker arm landed. The
//!   former aspirational ContractGap test has been replaced by a real
//!   smoke test (`multi_worker_emit_for_02_split_succeeds`) that
//!   exercises the actual emit path on a 2-worker fixture.
//!
//! What these tests pin:
//!
//! 1. Multi-worker `emit()` for a real 2-worker fixture succeeds and
//!    produces a `main.rs` containing the Ring<T> substrate +
//!    per-(DataId,SeqTag) ring allocations + at least one barrier +
//!    per-worker `thread::spawn` body.
//!
//! 2. Single-worker `emit()` succeeds for an empty event-list (the
//!    smallest legal input) AND produces a `main.rs` byte-identical
//!    to `pthreads_sync::emit`'s on the same input.
//!
//! 3. The `EmitResult` shape is the single-binary five-field shape
//!    (mirrors pthreads-sync). Compile-time only: if a later cycle
//!    changes the struct, this `let _r = EmitResult { ... }` won't
//!    compile and the driver dispatch arm has to be updated in lockstep.

use std::collections::BTreeMap;
use std::path::PathBuf;

use nucleus_compiler::event::{Event, WorkerId};
use nucleus_compiler::sidecar::NameSidecar;
use pthreads_async::{emit, EmitResult, NameTables};

/// Wave B-2 smoke test: the multi-worker emit path actually produces
/// a `main.rs` for a real 2-worker fixture (02-split-add/split). Pins
/// the four shape invariants Wave B-2 guarantees:
///
/// 1. `emit()` returns Ok (no ContractGap, no Plan::build error).
/// 2. The emitted `main.rs` contains the file-scope `Ring<T>` struct
///    via `emit_ring_struct_decl` (the substrate is wired).
/// 3. At least one `let ring_<id>: Arc<Ring<T>>` allocation appears
///    in `fn main` (the per-pair instances are emitted).
/// 4. At least one `bar_<id>: Arc<Barrier>` appears (02-split-add has
///    cross-worker writes, so `inject_syncs` injects barriers).
/// 5. At least one `_handle = thread::spawn(move || {` appears (the
///    non-host worker is spawned).
///
/// This is a UNIT test on the emit path — it does NOT cargo-build the
/// generated `main.rs`. End-to-end build/run/differential is TASK-0229.
#[test]
fn multi_worker_emit_for_02_split_succeeds() {
    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples/02-split-add");
    let algo_src = std::fs::read_to_string(ex.join("prog.algo.nuc")).expect("02 algo");
    let sched_src =
        std::fs::read_to_string(ex.join("schedules/split.sched.nuc")).expect("02 split sched");

    // 02-split-add/split is a 2-worker schedule (host + w_b) with
    // sync transfers (buffer=1). No partition=workers, no check_loop.
    let r = test_common::lower_for_test(
        &algo_src,
        &sched_src,
        &test_common::LowerForTestOpts::default(),
    );

    // TASK-0426.01: per-call-unique scratch (created once, never removed).
    let scratch = test_common::unique_scratch_dir(
        &root.join("nucleus/target/pthreads-async-test-scratch"),
        "multi_worker_02_split",
    );
    let result = emit(
        &r.per_worker,
        &r.names,
        &r.sidecar,
        &ex.join("kernels.rs"),
        &scratch,
    )
    .expect("multi-worker emit must succeed on 02-split-add/split");

    let main_rs = std::fs::read_to_string(&result.main_rs).expect("read main.rs");

    // (1) The Ring<T> substrate is present.
    assert!(
        main_rs.contains("struct Ring<T> {"),
        "multi-worker main.rs must contain the file-scope Ring<T> struct:\n{main_rs}"
    );
    assert!(
        main_rs.contains("fn push(&self, v: T) {"),
        "Ring<T> push method must be emitted:\n{main_rs}"
    );
    assert!(
        main_rs.contains("fn wait(&self) -> T {"),
        "Ring<T> wait method must be emitted:\n{main_rs}"
    );

    // (2) At least one per-pair Arc<Ring<T>> allocation.
    assert!(
        main_rs.contains("let ring_0: std::sync::Arc<Ring<"),
        "multi-worker main.rs must allocate at least one ring \
         (ring_0); not found in:\n{main_rs}"
    );

    // (3) At least one barrier allocation (02-split has cross-worker
    //     writes, so `inject_syncs` produces barriers).
    assert!(
        main_rs.contains(": Arc<Barrier> = Arc::new(Barrier::new("),
        "multi-worker main.rs must allocate at least one Barrier:\n{main_rs}"
    );

    // (4) At least one thread::spawn for the non-host worker.
    assert!(
        main_rs.contains("_handle = thread::spawn(move || {"),
        "multi-worker main.rs must spawn the non-host worker:\n{main_rs}"
    );

    // (5) The host joins every worker handle at fn main exit.
    assert!(
        main_rs.contains("_handle.join().expect("),
        "multi-worker main.rs must join every spawned handle:\n{main_rs}"
    );
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
    // TASK-0426.01: per-call-unique stem (created once by the helper,
    // never removed). The `async`/`sync` subdirs ride under this stem,
    // so write_file is deterministic without a prior-run remove.
    let stem = test_common::unique_scratch_dir(
        &target.join("pthreads-async-test-scratch"),
        "single_worker_empty",
    );
    let async_out = stem.join("async");
    let sync_out = stem.join("sync");

    // Both backends accept a `kernels.rs` that exists but is empty —
    // empty event list -> no kernel call site -> kernels.rs content
    // does not appear in main.rs. We use the same file for both
    // backends so the kernels.rs copy step in each is identical.
    let kernels = stem.join("empty_kernels.rs");
    std::fs::write(
        &kernels,
        "// Empty kernels.rs for the empty-eventlist test.\n",
    )
    .expect("empty kernels.rs");

    let async_res = emit(&per_worker, &names, &sidecar, &kernels, &async_out).expect("async emit");
    // Build pthreads_sync's NameTables from the re-exported alias.
    let sync_res =
        pthreads_sync::emit(&per_worker, &names, &sidecar, &kernels, &sync_out).expect("sync emit");

    // ---- The invariant: byte-identical main.rs, Cargo.toml, run.sh ----
    let async_main = std::fs::read_to_string(&async_res.main_rs).expect("async main.rs");
    let sync_main = std::fs::read_to_string(&sync_res.main_rs).expect("sync main.rs");
    assert_eq!(
        async_main, sync_main,
        "pthreads-async single-worker main.rs must be byte-identical \
         to pthreads-sync's (the cross-backend differential invariant): \n\
         async:\n{async_main}\n--- sync:\n{sync_main}"
    );

    let async_cargo = std::fs::read_to_string(&async_res.cargo_toml).expect("async Cargo.toml");
    let sync_cargo = std::fs::read_to_string(&sync_res.cargo_toml).expect("sync Cargo.toml");
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

    let async_kernels = std::fs::read_to_string(&async_res.kernels_rs).expect("async kernels.rs");
    let sync_kernels = std::fs::read_to_string(&sync_res.kernels_rs).expect("sync kernels.rs");
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
    (r.per_worker, r.names, r.sidecar, ex.join("kernels.rs"))
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

    // TASK-0426.01: per-call-unique scratch (created once, never removed).
    let scratch = test_common::unique_scratch_dir(
        &repo_root().join("nucleus/target/pthreads-async-test-scratch"),
        "single_worker_01_naive",
    );
    let async_out = scratch.join("async");
    let sync_out = scratch.join("sync");

    let async_res = emit(&per_worker, &names, &sidecar, &kernels, &async_out)
        .expect("pthreads-async emit (single-worker real example)");
    let sync_res = pthreads_sync::emit(&per_worker, &names, &sidecar, &kernels, &sync_out)
        .expect("pthreads-sync emit (same input)");

    // The cross-backend differential invariant: same algorithm + same
    // naive schedule -> byte-identical main.rs across both backends.
    let async_main = std::fs::read_to_string(&async_res.main_rs).expect("async main.rs");
    let sync_main = std::fs::read_to_string(&sync_res.main_rs).expect("sync main.rs");
    assert_eq!(
        async_main, sync_main,
        "pthreads-async single-worker main.rs on 01-elementwise-add/naive \
         MUST be byte-identical to pthreads-sync's (the cross-backend \
         differential invariant). A diff here means the delegation to \
         backend_common::single_worker_main::render_single_worker_main was bypassed or wrapped:\n\
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

// --------------------------------------------------------------------
// TASK-0245 (cycle 36): regression-pin for the cycle-35
// `render_int_expr` const-in-IndexExpr fix on the pthreads-async
// backend.
//
// Background: cycle 35 (commit 894f63f) fixed the PRIVATE
// `pthreads_sync::render_int_expr` to resolve declared consts (e.g.
// `ITERS`) when they appear inside an `IndexExpr`. pthreads-async
// consumes ALL of its multi-worker IndexExpr rendering through the
// SHARED `pthreads_sync::multi_worker_walker` (cycle 31 / TASK-0239)
// which routes through the pub shims (`render_fire_args_pub`,
// `render_const_expr_pub`); single-worker schedules delegate
// straight to `backend_common::single_worker_main::render_single_worker_main`.
//
// Cycle-36 audit grep
// (`grep -rnE "fn render_int_expr|fn render_flat_index|fn
// render_const_expr" nucleus/backends/pthreads-async/`) returned
// ZERO matches — confirming no parallel private renderer with its
// own consts gap exists. The cycle-31 dedup is what makes that
// audit clean; this test pins it structurally.
//
// What it pins:
//   1. The IndexExpr arithmetic site in the emitted `main.rs`
//      contains the resolved const LITERAL (`8`).
//   2. The bare const ident (`ITERS`) does NOT appear anywhere in
//      the emitted `main.rs`.
//
// Sibling tests for pthreads-sync + mp-tcp-bufsync live in
// `pthreads-sync/tests/multi_worker.rs` + `mp-tcp-bufsync/tests/
// pingpong.rs`, all driven by the same `test_common::CONST_IN_INDEXEXPR_*`
// fixture (single source of truth).
//
// Emit-string only — no cargo-build. End-to-end correctness is
// already covered by the e2e gate on example 11 × pthreads-async
// (cycle-35 PASS bit-identical evidence).
#[test]
fn const_in_indexexpr_pthreads_async_resolves_to_literal_value() {
    let r = test_common::lower_for_test(
        test_common::CONST_IN_INDEXEXPR_ALGO_SRC,
        test_common::CONST_IN_INDEXEXPR_SCHED_SRC,
        &test_common::LowerForTestOpts::default(),
    );

    // TASK-0426.01: per-call-unique scratch (created once by the helper,
    // never removed). The `gen` subdir rides under this unique scratch.
    let scratch = test_common::unique_scratch_dir(
        &repo_root().join("nucleus/target/pthreads-async-test-scratch"),
        "const_in_indexexpr_pthreads_async",
    );
    let kernels_path = scratch.join("kernels.rs");
    std::fs::write(&kernels_path, "// stub for emit-string test\n").unwrap();

    let result = emit(
        &r.per_worker,
        &r.names,
        &r.sidecar,
        &kernels_path,
        &scratch.join("gen"),
    )
    .expect("pthreads-async emit must succeed on const-in-IndexExpr fixture");
    let main_rs = std::fs::read_to_string(&result.main_rs).expect("read main.rs");

    let iters_val = test_common::CONST_IN_INDEXEXPR_ITERS_VALUE;
    let resolved_row = format!("({iters_val}) * 4");
    let bare_ident = test_common::CONST_IN_INDEXEXPR_ITERS_IDENT;

    // (1) The resolved literal `8` appears at the IndexExpr site.
    assert!(
        main_rs.contains(&resolved_row),
        "pthreads-async main.rs must contain the resolved `ITERS=8` literal at \
         the IndexExpr row-stride site (`{resolved_row}`); cycle-35 fix not \
         reaching this code path via multi_worker_walker. main.rs:\n{main_rs}"
    );

    // (2) The bare const ident `ITERS` does NOT appear anywhere in
    // the emitted main.rs.
    assert!(
        !main_rs.contains(bare_ident),
        "pthreads-async main.rs must NOT contain the bare const ident \
         `{bare_ident}` — pthreads-async consumes IndexExpr rendering through \
         pthreads_sync's pub shims (cycle-31 dedup); the cycle-35 fix MUST \
         reach this backend (TASK-0245 audit). main.rs:\n{main_rs}"
    );
}
