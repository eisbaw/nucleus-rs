//! mp-tcp-bufsync emit-string assertion test for `check loop V :
//! latency_max=T` (TASK-0052.02 review-gate finding number 2).
//!
//! Why this file exists: the implementer's main commit `d2bbf76`
//! wired identical Instant-now-then-duration-check-then-panic codegen
//! into the mp-tcp-bufsync `Event::Loop` arm to mirror pthreads-sync
//! (`nucleus/backends/pthreads-sync/tests/check_frame_codegen.rs`),
//! but there was
//! no test pinning the mp-tcp emit string. AC number 1 of TASK-0052.02
//! lists BOTH backends as required and the cross-backend claim was
//! untested until this file landed (architect review finding number 2
//! on the cycle, fixed in-thread).
//!
//! This test does NOT run the generated binary; it only confirms the
//! `Event::Loop` codegen path emits the contracted shape. The
//! single-worker schedule below triggers mp-tcp's
//! single-process-fused-into-one-binary emit; the `worker_bins[0]`
//! path is the rendered source.

use std::fs;
use std::path::{Path, PathBuf};

// Cycle-25 (TASK-0238): the pipeline imports were used by the
// pre-refactor inline helpers; both helpers now call
// test_common::lower_for_test which encapsulates them. Only NameTables
// remains.
use pthreads_sync::NameTables;

fn repo_root() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .expect("three ancestors above mp-tcp-bufsync crate")
        .to_path_buf()
}

fn scratch_dir(name: &str) -> PathBuf {
    // TASK-0426: each call gets a process-and-call-unique subdir
    // (`{name}-{pid}-{counter}`) so concurrent test threads NEVER share a
    // path. The old code reused a fixed `target/{name}` and did
    // remove_dir_all + create_dir_all on it; under parallel `cargo test`
    // that remove/create dance raced with a sibling test's `fs::write`,
    // intermittently surfacing ENOENT (NotFound) in `write_kernels_stub`.
    // A per-call-unique path that is created once (never removed/recreated)
    // removes the shared mutable filesystem state the race needs.
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let target = repo_root().join("nucleus/target/mp-tcp-bufsync-check-frame-scratch");
    let _ = fs::create_dir_all(&target);
    let nonce = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = target.join(format!("{name}-{}-{}", std::process::id(), nonce));
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn build_per_worker(
    algo_src: &str,
    sched_src: &str,
) -> (
    std::collections::BTreeMap<
        nucleus_compiler::event::WorkerId,
        Vec<nucleus_compiler::event::Event>,
    >,
    NameTables,
    nucleus_compiler::sidecar::NameSidecar,
) {
    // Cycle-25 follow-on (cycle-24 review-gate A.3 LOW closed): the
    // pre-cycle-24 single-worker variant of this helper was NOT
    // refactored to call test_common; cycle 25's TASK-0238 made the
    // refactor cleaner (LowerForTestResult.names is now a pre-built
    // NameTables). The 4th in-file site is now one call.
    //
    // Single-worker check_frame schedules need inject_check_frames=true
    // but NOT apply_partition_workers (single worker) and NOT
    // apply_block_transforms (pre-cycle-24 helper didn't call it; mirror
    // that pre-refactor byte-faithful behaviour, even though
    // block_transforms is a no-op on these schedules).
    let r = test_common::lower_for_test(
        algo_src,
        sched_src,
        &test_common::LowerForTestOpts {
            apply_block_transforms: false,
            apply_partition_workers: false,
            inject_check_frames: true,
        },
    );
    (r.per_worker, r.names, r.sidecar)
}

#[test]
fn mp_tcp_bufsync_emit_includes_panic_instrumentation_on_check_loop() {
    // TASK-0052.02 review-gate finding #2: the mp-tcp emit path
    // landed in commit d2bbf76 was untested. This test pins the
    // emit-string shape so a regression is caught.
    let algo = "\
const N : usize = 4;
data a : i32[N];
data c : i32[N];
kernel load_input  : ()      -> i32[N] effectful;
kernel save_output : (i32[N]) -> () effectful;
kernel inc : (i32) -> i32 pure;
a <-- load_input();
for n : 0 .. N {
    c[n] <-- inc(a[n]);
}
save_output(c);
";
    let sched = "\
schedule for \"a.algo.nuc\" {
    workers = { host };
    place load_input on host;
    place save_output on host;
    place inc on host;
    check loop n : latency_max = 1ms;
}
";
    let (per_worker, names, sidecar) = build_per_worker(algo, sched);

    let out = scratch_dir("mp_tcp_panic_codegen");
    let kernels_rs = out.join("kernels_stub.rs");
    fs::write(
        &kernels_rs,
        "pub fn load_input() -> Vec<i32> { vec![0; 4] }\n\
         pub fn save_output(_c: Vec<i32>) {}\n\
         pub fn inc(x: i32) -> i32 { x + 1 }\n",
    )
    .expect("write kernels stub");

    let res = mp_tcp_bufsync::emit(&per_worker, &names, &sidecar, &kernels_rs, &out)
        .expect("emit must succeed for a valid schedule with a check loop");

    // mp-tcp emits per-worker bin files; for a single-worker schedule
    // there is one entry.
    assert!(
        !res.worker_bins.is_empty(),
        "expected at least one worker_bin; got none"
    );
    let bin_src = fs::read_to_string(&res.worker_bins[0]).expect("read worker bin");

    assert!(
        bin_src.contains("let _check_start = std::time::Instant::now();"),
        "worker bin must contain Instant::now() inside the loop body. Got:\n{bin_src}"
    );
    assert!(
        bin_src.contains("let _check_elapsed = _check_start.elapsed().as_nanos();"),
        "worker bin must contain the elapsed-nanos read. Got:\n{bin_src}"
    );
    assert!(
        bin_src.contains("if _check_elapsed > 1000000_u128"),
        "worker bin must contain `if _check_elapsed > 1000000_u128`. Got:\n{bin_src}"
    );
    assert!(
        bin_src.contains("panic!(\"latency budget violated on `check loop n`"),
        "panic message must name the loop_var `n`. Got:\n{bin_src}"
    );
    assert!(
        bin_src.contains("max 1000000 ns"),
        "panic message must contain `max 1000000 ns`. Got:\n{bin_src}"
    );
}

#[test]
fn mp_tcp_bufsync_emit_unchanged_without_check_loop_directive() {
    // Sister-test of the pthreads-sync variant: a schedule WITHOUT
    // `check loop V` must NOT emit Instant::now(). Determinism
    // contract: zero byte change when no check directive is present.
    let algo = "\
const N : usize = 4;
data a : i32[N];
data c : i32[N];
kernel load_input  : ()      -> i32[N] effectful;
kernel save_output : (i32[N]) -> () effectful;
kernel inc : (i32) -> i32 pure;
a <-- load_input();
for n : 0 .. N {
    c[n] <-- inc(a[n]);
}
save_output(c);
";
    let sched = "\
schedule for \"a.algo.nuc\" {
    workers = { host };
    place load_input on host;
    place save_output on host;
    place inc on host;
}
";
    let (per_worker, names, sidecar) = build_per_worker(algo, sched);

    let out = scratch_dir("mp_tcp_no_check");
    let kernels_rs = out.join("kernels_stub.rs");
    fs::write(
        &kernels_rs,
        "pub fn load_input() -> Vec<i32> { vec![0; 4] }\n\
         pub fn save_output(_c: Vec<i32>) {}\n\
         pub fn inc(x: i32) -> i32 { x + 1 }\n",
    )
    .expect("write kernels stub");

    let res = mp_tcp_bufsync::emit(&per_worker, &names, &sidecar, &kernels_rs, &out)
        .expect("emit must succeed");
    assert!(!res.worker_bins.is_empty());
    let bin_src = fs::read_to_string(&res.worker_bins[0]).expect("read worker bin");

    assert!(
        !bin_src.contains("Instant::now"),
        "worker bin MUST NOT contain Instant::now() when no check loop directive. Got:\n{bin_src}"
    );
    assert!(
        !bin_src.contains("_check_start"),
        "worker bin MUST NOT contain _check_start. Got:\n{bin_src}"
    );
}

// --------------------------------------------------------------------
// TASK-0052.04: on_violation=log + on_violation=count.
// Same emit-string shape as pthreads-sync (single codegen
// implementation, shared helpers `collect_count_check_frames` +
// `emit_count_reporter_struct` + `sanitize_loop_var`). The two
// suites pin the same patterns on each backend so a drift fails
// loudly in one of them.
// --------------------------------------------------------------------

#[test]
fn mp_tcp_bufsync_emit_includes_log_eprintln_on_check_loop() {
    let algo = "\
const N : usize = 4;
data a : i32[N];
data c : i32[N];
kernel load_input  : ()      -> i32[N] effectful;
kernel save_output : (i32[N]) -> () effectful;
kernel inc : (i32) -> i32 pure;
a <-- load_input();
for n : 0 .. N {
    c[n] <-- inc(a[n]);
}
save_output(c);
";
    let sched = "\
schedule for \"a.algo.nuc\" {
    workers = { host };
    place load_input on host;
    place save_output on host;
    place inc on host;
    check loop n : latency_max = 1ms, on_violation = log;
}
";
    let (per_worker, names, sidecar) = build_per_worker(algo, sched);

    let out = scratch_dir("mp_tcp_log_codegen");
    let kernels_rs = out.join("kernels_stub.rs");
    fs::write(
        &kernels_rs,
        "pub fn load_input() -> Vec<i32> { vec![0; 4] }\n\
         pub fn save_output(_c: Vec<i32>) {}\n\
         pub fn inc(x: i32) -> i32 { x + 1 }\n",
    )
    .expect("write kernels stub");

    let res = mp_tcp_bufsync::emit(&per_worker, &names, &sidecar, &kernels_rs, &out)
        .expect("emit must succeed");
    assert!(!res.worker_bins.is_empty());
    let bin_src = fs::read_to_string(&res.worker_bins[0]).expect("read worker bin");

    assert!(
        bin_src.contains("let _check_start = std::time::Instant::now();"),
        "Log path must still emit Instant measurement. Got:\n{bin_src}"
    );
    assert!(
        bin_src.contains("eprintln!(\"warning: check loop `n` violated latency_max=1000000 ns:"),
        "Log path must emit the eprintln warning. Got:\n{bin_src}"
    );
    assert!(
        !bin_src.contains("panic!(\"latency budget violated"),
        "Log path must NOT emit panic!. Got:\n{bin_src}"
    );
    assert!(
        !bin_src.contains("NUC_CHECK_COUNT_"),
        "Log-only schedule must not emit Count statics. Got:\n{bin_src}"
    );
}

#[test]
fn mp_tcp_bufsync_emit_includes_atomic_and_reporter_on_count_violation() {
    let algo = "\
const N : usize = 4;
data a : i32[N];
data c : i32[N];
kernel load_input  : ()      -> i32[N] effectful;
kernel save_output : (i32[N]) -> () effectful;
kernel inc : (i32) -> i32 pure;
a <-- load_input();
for n : 0 .. N {
    c[n] <-- inc(a[n]);
}
save_output(c);
";
    let sched = "\
schedule for \"a.algo.nuc\" {
    workers = { host };
    place load_input on host;
    place save_output on host;
    place inc on host;
    check loop n : latency_max = 1ms, on_violation = count;
}
";
    let (per_worker, names, sidecar) = build_per_worker(algo, sched);

    let out = scratch_dir("mp_tcp_count_codegen");
    let kernels_rs = out.join("kernels_stub.rs");
    fs::write(
        &kernels_rs,
        "pub fn load_input() -> Vec<i32> { vec![0; 4] }\n\
         pub fn save_output(_c: Vec<i32>) {}\n\
         pub fn inc(x: i32) -> i32 { x + 1 }\n",
    )
    .expect("write kernels stub");

    let res = mp_tcp_bufsync::emit(&per_worker, &names, &sidecar, &kernels_rs, &out)
        .expect("emit must succeed");
    let bin_src = fs::read_to_string(&res.worker_bins[0]).expect("read worker bin");

    assert!(
        bin_src.contains(
            "static NUC_CHECK_COUNT_n: std::sync::atomic::AtomicU64 = \
             std::sync::atomic::AtomicU64::new(0);"
        ),
        "Count path must emit the AtomicU64 static at file scope. Got:\n{bin_src}"
    );
    assert!(
        bin_src.contains("struct NucCheckCountReporter {"),
        "Count path must emit the reporter struct. Got:\n{bin_src}"
    );
    assert!(
        bin_src.contains("impl Drop for NucCheckCountReporter {"),
        "Count path must emit the Drop impl. Got:\n{bin_src}"
    );
    assert!(
        bin_src.contains("let _nuc_check_reporter_n = NucCheckCountReporter {"),
        "main must instantiate the per-loop guard local. Got:\n{bin_src}"
    );
    assert!(
        bin_src.contains(
            "if _check_elapsed > 1000000_u128 { \
             NUC_CHECK_COUNT_n.fetch_add(1, std::sync::atomic::Ordering::Relaxed); }"
        ),
        "loop body must fetch_add on threshold violation. Got:\n{bin_src}"
    );
    assert!(
        !bin_src.contains("panic!(\"latency budget violated"),
        "Count path must NOT emit panic!. Got:\n{bin_src}"
    );
}

// --------------------------------------------------------------------
// TASK-0236 (cycle 23, 2026-05-22): MULTI-WORKER check_frame emit-string
// pinning for Log + Count on_violation kinds. Mirror of the
// pthreads-sync multi_worker.rs Log + Count tests; closes the cycle-22
// review-gate B.1 gap (multi-worker emit paths were byte-transparent
// only by shared-helper construction, not by direct test).
//
// These tests exercise the MULTI-WORKER `render_worker_program` arm in
// mp-tcp-bufsync, which writes a per-worker `src/bin/<worker>.rs`. The
// 4 shared templates from TASK-0222 are called from inside that
// per-worker render path; this is what the cycle-22 architect review
// flagged as having no direct pinning coverage.
// --------------------------------------------------------------------

/// Variant of build_per_worker that also runs apply_partition_workers
/// plus inject_check_frames (needed for `partition=workers` schedules
/// with check_loop directives).
///
/// TASK-0237 (cycle 24): the lower-link-inject pipeline now lives in
/// the shared `test_common::lower_for_test` helper. Thin glue here
/// composes the local NameTables type from the result.
fn build_per_worker_partitioned(
    algo_src: &str,
    sched_src: &str,
) -> (
    std::collections::BTreeMap<
        nucleus_compiler::event::WorkerId,
        Vec<nucleus_compiler::event::Event>,
    >,
    NameTables,
    nucleus_compiler::sidecar::NameSidecar,
) {
    let r = test_common::lower_for_test(
        algo_src,
        sched_src,
        &test_common::LowerForTestOpts {
            apply_block_transforms: true,
            apply_partition_workers: true,
            inject_check_frames: true,
        },
    );
    (r.per_worker, r.names, r.sidecar)
}

const MULTI_ALGO_SRC: &str = "\
const N : usize = 4;
data x : i32[N];
data y : i32[N];
kernel load_input  : ()      -> i32[N] effectful;
kernel save_output : (i32[N]) -> () effectful;
kernel slow_inc    : (i32)    -> i32   pure;
x <-- load_input();
for n : 0 .. N {
    y[n] <-- slow_inc(x[n]);
}
save_output(y);
";

fn write_kernels_stub(out: &Path) -> PathBuf {
    let kernels_rs = out.join("kernels_stub.rs");
    fs::write(
        &kernels_rs,
        "pub fn load_input() -> Vec<i32> { vec![0; 4] }\n\
         pub fn save_output(_c: Vec<i32>) {}\n\
         pub fn slow_inc(x: i32) -> i32 { x + 1 }\n",
    )
    .expect("write kernels stub");
    kernels_rs
}

#[test]
fn mp_tcp_bufsync_multi_worker_log_emit_pins_per_thread_eprintln_template() {
    let sched = "\
schedule for \"a.algo.nuc\" {
    workers = { host, w0, w1 };
    place load_input  on host;
    place save_output on host;
    place slow_inc    on { w0, w1 };
    loop n : partition=workers;
    transfer x : sync;
    transfer y : sync;
    check loop n : latency_max = 5ms, on_violation = log;
}
";
    let (per_worker, names, sidecar) = build_per_worker_partitioned(MULTI_ALGO_SRC, sched);
    let out = scratch_dir("mp_tcp_multi_worker_log_emit");
    let kernels_rs = write_kernels_stub(&out);
    let res = mp_tcp_bufsync::emit(&per_worker, &names, &sidecar, &kernels_rs, &out)
        .expect("multi-worker emit with on_violation=log must succeed");

    // mp-tcp emits one bin per used worker. Expect EXACTLY 3 (host +
    // w0 + w1); tightened from `>= 2` per cycle-23 review-gate C.1 —
    // the commit body claims 3 bins so the assert must pin that count.
    assert_eq!(
        res.worker_bins.len(),
        3,
        "multi-worker emit must produce exactly 3 worker bins (host + w0 + w1); got {}",
        res.worker_bins.len()
    );

    // The eprintln template appears in EACH of w0's and w1's bin
    // (host's bin has no check_frame because partition=workers projects
    // the loop onto compute workers only).
    let mut eprintln_count = 0usize;
    for bin in &res.worker_bins {
        let bin_src = fs::read_to_string(bin).expect("read worker bin");
        let n = bin_src
            .matches(
                "eprintln!(\"warning: check loop `n` violated latency_max=5000000 ns: iteration took {} ns\", _check_elapsed);",
            )
            .count();
        eprintln_count += n;
    }
    assert_eq!(
        eprintln_count, 2,
        "expected exactly 2 Log eprintln sites across all worker bins (one per \
         partitioned worker); got {eprintln_count}. Shared template via \
         emit_log_branch (TASK-0222) must produce SAME template across all \
         consumers."
    );
}

#[test]
fn mp_tcp_bufsync_multi_worker_count_emit_pins_static_guard_and_fetch_add() {
    let sched = "\
schedule for \"a.algo.nuc\" {
    workers = { host, w0, w1 };
    place load_input  on host;
    place save_output on host;
    place slow_inc    on { w0, w1 };
    loop n : partition=workers;
    transfer x : sync;
    transfer y : sync;
    check loop n : latency_max = 5ms, on_violation = count;
}
";
    let (per_worker, names, sidecar) = build_per_worker_partitioned(MULTI_ALGO_SRC, sched);
    let out = scratch_dir("mp_tcp_multi_worker_count_emit");
    let kernels_rs = write_kernels_stub(&out);
    let res = mp_tcp_bufsync::emit(&per_worker, &names, &sidecar, &kernels_rs, &out)
        .expect("multi-worker emit with on_violation=count must succeed");

    let mut total_static_count = 0usize;
    let mut total_guard_count = 0usize;
    let mut total_fetch_add = 0usize;
    let mut reporter_struct_seen = 0usize;
    for bin in &res.worker_bins {
        let bin_src = fs::read_to_string(bin).expect("read worker bin");
        total_static_count += bin_src.matches(
            "static NUC_CHECK_COUNT_n: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);"
        ).count();
        total_guard_count += bin_src
            .matches("let _nuc_check_reporter_n = NucCheckCountReporter {")
            .count();
        total_fetch_add += bin_src
            .matches("NUC_CHECK_COUNT_n.fetch_add(1, std::sync::atomic::Ordering::Relaxed);")
            .count();
        if bin_src.contains("struct NucCheckCountReporter {") {
            reporter_struct_seen += 1;
        }
    }

    // Each worker bin is a SEPARATE PROCESS (mp-tcp-bufsync). Unlike
    // pthreads-sync's multi-worker (one process, shared static), every
    // worker process owns its OWN static + guard + reporter struct.
    // So each compute worker (w0, w1) has 1 static + 1 guard + 1
    // fetch_add + 1 reporter struct = 2 of each across the 2 compute
    // workers' bins. Host's bin has none (not a participant).
    assert_eq!(
        total_static_count, 2,
        "expected 2 static AtomicU64 across workers (one per compute worker process); \
         got {total_static_count}"
    );
    assert_eq!(
        total_guard_count, 2,
        "expected 2 guard locals across workers (per-process); got {total_guard_count}"
    );
    assert_eq!(
        total_fetch_add, 2,
        "expected 2 fetch_add sites across workers (per-process); got {total_fetch_add}"
    );
    assert_eq!(
        reporter_struct_seen, 2,
        "expected 2 NucCheckCountReporter struct definitions (per-process); \
         got {reporter_struct_seen}"
    );
}

#[test]
fn mp_tcp_bufsync_multi_worker_panic_emit_pins_per_worker_panic_template() {
    // Cycle-23 review-gate HIGH A.3 finding: TASK-0236 landed Log +
    // Count multi-worker pins for both backends, but pthreads-sync had
    // a Panic multi-worker pin from cycle 16 (TASK-0052.05) that
    // mp-tcp-bufsync lacked. Without this test, the cross-backend
    // ViolationKind coverage was 3/3 for pthreads-sync but only 2/3
    // for mp-tcp-bufsync. Closing the symmetry gap.
    //
    // Emit-only (no cargo build/run): mirrors the cycle-23 Log + Count
    // shape; the single-worker Panic test
    // (mp_tcp_bufsync_emit_includes_panic_instrumentation_on_check_loop
    // above) already exercises the build+run path on the single-worker
    // arm; this test pins the emit-string shape for the MULTI-WORKER
    // arm specifically.
    let sched = "\
schedule for \"a.algo.nuc\" {
    workers = { host, w0, w1 };
    place load_input  on host;
    place save_output on host;
    place slow_inc    on { w0, w1 };
    loop n : partition=workers;
    transfer x : sync;
    transfer y : sync;
    check loop n : latency_max = 5ms, on_violation = panic;
}
";
    let (per_worker, names, sidecar) = build_per_worker_partitioned(MULTI_ALGO_SRC, sched);
    let out = scratch_dir("mp_tcp_multi_worker_panic_emit");
    let kernels_rs = write_kernels_stub(&out);
    let res = mp_tcp_bufsync::emit(&per_worker, &names, &sidecar, &kernels_rs, &out)
        .expect("multi-worker emit with on_violation=panic must succeed");

    // 3 worker bins: host + w0 + w1. Tightened from `>= 2` to `== 3`
    // per cycle-23 review-gate C.1 (the commit body claims 3 bins; the
    // assertion should pin it).
    assert_eq!(
        res.worker_bins.len(),
        3,
        "multi-worker emit must produce exactly 3 worker bins (host + w0 + w1); got {}",
        res.worker_bins.len()
    );

    let mut panic_count = 0usize;
    for bin in &res.worker_bins {
        let bin_src = fs::read_to_string(bin).expect("read worker bin");
        panic_count += bin_src
            .matches("panic!(\"latency budget violated on `check loop n`")
            .count();
    }
    // 2 panic sites (one per compute worker; host has no check_frame
    // under partition=workers — same invariant as Log/Count tests).
    assert_eq!(
        panic_count, 2,
        "expected exactly 2 Panic sites across worker bins (per-compute-worker, \
         host excluded under partition=workers); got {panic_count}"
    );
}
