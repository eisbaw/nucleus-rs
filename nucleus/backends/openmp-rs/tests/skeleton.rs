//! Smoke tests for the openmp-rs backend.
//!
//! Cycle status (TASK-0044.01.01 cycle 196).
//!
//! Single-worker arm IMPLEMENTED (cycle 191): delegates to
//! `backend_common::single_worker_main::render_single_worker_main` +
//! `backend_common::project_skeleton::single_binary` with
//! `extra_dependencies = None`.
//!
//! Multi-worker arm IMPLEMENTED (cycle 196): rayon::scope spawn site
//! plus verbatim Slot<T> / Arc<Barrier> rendezvous from pthreads-sync;
//! emitted Cargo.toml carries `rayon = "1"` via
//! `extra_dependencies = Some(_)`.
//!
//! Tests pinned here:
//! 1. The `EmitResult` struct shape (compile-pin via constructor).
//! 2. Multi-worker `emit()` succeeds and emits the load-bearing
//!    rayon-substrate needles (`rayon::scope`, `rayon = "1"` in
//!    Cargo.toml) + the anti-needle (`std::thread` MUST NOT appear
//!    in the multi-worker `use` block).
//!
//! Bit-identical OUTPUT-vs-pthreads-sync coverage lives in
//! `tests/multi_worker_emit.rs` (structural-diff oracle) and the
//! e2e-matrix bit-identical cells (runtime falsifiability).

use std::collections::BTreeMap;
use std::path::PathBuf;

use nucleus_compiler::event::{Event, WorkerId};
use openmp_rs::{emit, EmitResult, NameTables};

/// TASK-0044.01.01 cycle 196: multi-worker emit now SUCCEEDS (the
/// cycle-191 ContractGap pin is replaced by this success pin). The
/// emit MUST: (a) succeed via the Ok arm; (b) contain `rayon::scope`
/// in the emitted main.rs (the substrate swap positive needle);
/// (c) NOT contain `use std::thread;` or `thread::spawn(` (the
/// substrate swap anti-needle — a regression to pthreads-sync's
/// substrate would re-introduce these); (d) emit `rayon = "1"` in
/// the Cargo.toml `[dependencies]` block so the generated project
/// builds standalone.
///
/// The anti-needle is the cycle-195b oracle lesson: assert that the
/// substrate-swap is present BEFORE any canonicaliser, otherwise a
/// regression to thread::spawn could silently no-op the swap and pass
/// the byte-equivalence test. The full structural-diff vs pthreads-sync
/// lives in `tests/multi_worker_emit.rs`.
#[test]
fn multi_worker_emit_uses_rayon_scope_not_thread_spawn() {
    use test_common::{lower_for_test, LowerForTestOpts};

    let root = repo_root();
    let ex = root.join("nuc-nucleus/examples/02-split-add");
    let algo_src = std::fs::read_to_string(ex.join("prog.algo.nuc")).expect("02 algo");
    let sched_src =
        std::fs::read_to_string(ex.join("schedules/split.sched.nuc")).expect("02 split sched");

    // 02-split-add/split is the simplest in-tree 2-worker schedule
    // (host + w0). Default opts (apply_block_transforms=true) — same
    // as the pthreads-sync e2e cell's pipeline shape.
    let r = lower_for_test(&algo_src, &sched_src, &LowerForTestOpts::default());

    // TASK-0426.01: per-call-unique scratch (created once, never removed).
    let scratch = test_common::unique_scratch_dir(
        &root.join("nucleus/target/openmp-rs-test-scratch"),
        "multi_worker_smoke_02_split",
    );
    let kernels = ex.join("kernels.rs");

    let result = emit(&r.per_worker, &r.names, &r.sidecar, &kernels, &scratch)
        .expect("openmp-rs multi-worker emit must succeed (cycle 196)");
    let main_rs = std::fs::read_to_string(&result.main_rs).expect("read main.rs");
    let cargo_toml = std::fs::read_to_string(&result.cargo_toml).expect("read Cargo.toml");

    // (a) positive needle: rayon::scope substrate is the load-bearing
    // distinction vs pthreads-sync.
    assert!(
        main_rs.contains("rayon::scope(|s| {"),
        "openmp-rs multi-worker main.rs must use rayon::scope; got:\n{main_rs}"
    );
    assert!(
        main_rs.contains("s.spawn(move |_| {"),
        "openmp-rs multi-worker main.rs must use s.spawn(move |_| ...); got:\n{main_rs}"
    );

    // (b) anti-needle: a regression to std::thread::spawn would mean
    // the multi_worker arm accidentally delegated to pthreads-sync's
    // emitter (e.g. via a copy-paste). Bite at the source layer so
    // the cycle-195b silent-no-op canonicaliser pattern can't fire.
    assert!(
        !main_rs.contains("use std::thread"),
        "openmp-rs multi-worker MUST NOT import std::thread (cycle-196 substrate swap); got:\n{main_rs}"
    );
    assert!(
        !main_rs.contains("thread::spawn"),
        "openmp-rs multi-worker MUST NOT call thread::spawn (cycle-196 substrate swap); got:\n{main_rs}"
    );
    assert!(
        !main_rs.contains(".join().expect(\"worker thread panicked\")"),
        "openmp-rs multi-worker MUST NOT carry pthreads-sync's join loop (rayon::scope has implicit join); got:\n{main_rs}"
    );

    // (c) Cargo.toml carries `rayon = "1"`.
    assert!(
        cargo_toml.contains("[dependencies]"),
        "openmp-rs multi-worker Cargo.toml must have a [dependencies] section; got:\n{cargo_toml}"
    );
    assert!(
        cargo_toml.contains("rayon = \"1\""),
        "openmp-rs multi-worker Cargo.toml must declare rayon = \"1\"; got:\n{cargo_toml}"
    );
}

#[test]
fn emit_result_shape_is_single_binary_five_field() {
    // Compile-time + minimal-runtime pin of the EmitResult shape. If a
    // later cycle adds / renames / removes a field, this test fails
    // and the driver dispatch arm (driver/src/main.rs match-arm
    // "openmp-rs" => { ... println!(...) ... }) MUST be updated in
    // lockstep — the same five println! lines (project_dir / cargo_toml
    // / main_rs / kernels_rs / run_sh) the dispatch site prints.
    let r = EmitResult {
        project_dir: PathBuf::from("/p"),
        cargo_toml: PathBuf::from("/p/Cargo.toml"),
        main_rs: PathBuf::from("/p/src/main.rs"),
        kernels_rs: PathBuf::from("/p/src/kernels.rs"),
        run_sh: PathBuf::from("/p/run.sh"),
    };
    let _ = (
        &r.project_dir,
        &r.cargo_toml,
        &r.main_rs,
        &r.kernels_rs,
        &r.run_sh,
    );
}

/// Empty per-worker map MUST still succeed via the single-worker arm
/// (used_workers.len() == 0 -> dispatch to pthreads-sync's straight-line
/// emitter; Cargo.toml `extra_dependencies = None` keeps the bytes
/// identical to pthreads-sync's). Mirrors pthreads-sync's empty-event
/// behavioural pin.
#[test]
fn empty_per_worker_succeeds_via_single_worker_arm() {
    let per_worker: BTreeMap<WorkerId, Vec<Event>> = BTreeMap::new();
    let names = NameTables::default();
    let sidecar = nucleus_compiler::sidecar::NameSidecar::default();

    // TASK-0426.01: per-call-unique scratch (created once by the helper,
    // never removed) — kills the remove/create race class.
    let scratch = test_common::unique_scratch_dir(
        &repo_root().join("nucleus/target/openmp-rs-test-scratch"),
        "empty_per_worker",
    );
    let kernels = scratch.join("kernels.rs");
    std::fs::write(&kernels, "// empty\n").expect("kernels stub");

    let result = emit(
        &per_worker,
        &names,
        &sidecar,
        &kernels,
        &scratch.join("out"),
    )
    .expect("empty per_worker must succeed (single-worker arm)");
    let cargo_toml = std::fs::read_to_string(&result.cargo_toml).expect("read Cargo.toml");
    // No rayon dep on the single-worker arm — byte-identical to
    // pthreads-sync's Cargo.toml.
    assert!(
        !cargo_toml.contains("rayon"),
        "single-worker openmp-rs Cargo.toml MUST NOT contain rayon dep (byte-identity vs pthreads-sync); got:\n{cargo_toml}"
    );
}

fn repo_root() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(std::path::Path::parent)
        .and_then(std::path::Path::parent)
        .expect("three ancestors above openmp-rs crate")
        .to_path_buf()
}
