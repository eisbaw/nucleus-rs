//! pthreads-async emit-string assertion tests for `check loop V :
//! latency_max=T` on the MULTI-WORKER arm (TASK-0227 + TASK-0240,
//! cycle 29 — paired closure).
//!
//! Why this file exists: cycle 26 (TASK-0228 Wave B-2) landed the
//! multi-worker check_frame substrate in `pthreads_async::multi_worker
//! ::Plan::emit` + `render_worker_events`. The shared helpers from
//! pthreads-sync (`emit_count_static`, `emit_count_reporter_struct`,
//! `emit_count_guard_local`, `emit_log_branch`, `emit_count_branch`,
//! `collect_count_check_frames`, `sanitize_loop_var`) are CALLED from
//! the codegen path, but no in-tree multi-worker pthreads-async
//! fixture carried a `check loop V` directive — so the emit-string
//! shape was structurally byte-transparent (by shared-helper
//! construction) but NOT test-pinned for THIS backend. Architect
//! review-gate in cycle 26 un-ticked TASK-0228 AC#5 over AC-gaming
//! concerns and filed TASK-0240 to close the gap.
//!
//! Scope: MULTI-WORKER only. Single-worker check_frame on pthreads-
//! async is inherited free via the cycle-17 delegation to
//! `pthreads_sync::render_single_worker_main` (TASK-0226) — the
//! single-worker emit-string is byte-identical to pthreads-sync's, so
//! pinning it again here would be redundant with
//! `pthreads-sync/tests/check_frame_codegen.rs`. This file exclusively
//! exercises the multi-worker render_worker_events Event::Loop arm
//! with check_frame populated.
//!
//! Shared-memory model contrast (vs mp-tcp-bufsync): pthreads-async
//! emits ONE single binary with N spawned threads, so the Count
//! substrate is SHARED across threads — one `static AtomicU64`, one
//! Drop-guard local on the host thread, N `fetch_add` sites (one per
//! compute worker). mp-tcp-bufsync emits per-process bins so each
//! process has its OWN static + guard + reporter struct (the cycle 23
//! TASK-0236 mp-tcp test pins per-process counts of 2). The test
//! expectations here therefore mirror pthreads-sync's
//! `multi_worker_check_loop_count_emit_*` shape (file-scope dedup),
//! NOT mp-tcp's.

use std::fs;
use std::path::{Path, PathBuf};

use pthreads_async::emit;
use pthreads_sync::NameTables;

/// Locate the workspace root (three ancestors up from the
/// pthreads-async crate's CARGO_MANIFEST_DIR — nucleus/backends/
/// pthreads-async → nucleus/backends → nucleus → repo root).
fn repo_root() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .expect("three ancestors above pthreads-async crate")
        .to_path_buf()
}

/// Per-test scratch dir (TASK-0241 forward-carry): each test must use
/// a UNIQUE scratch dir to avoid `remove_dir_all` of test A racing the
/// emit-then-read of test B under cargo's parallel test runner. The
/// name is the test function name; dir is wiped before recreating.
fn scratch_dir(name: &str) -> PathBuf {
    let target = repo_root().join("nucleus/target/pthreads-async-check-frame-scratch");
    let _ = fs::create_dir_all(&target);
    let dir = target.join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Shared algorithm source for the three multi-worker check_frame
/// tests. Same shape as pthreads-sync's `CHECK_ALGO_SRC` and
/// mp-tcp-bufsync's `MULTI_ALGO_SRC`: one parameterised loop over N
/// that the schedule will partition across two compute workers.
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

/// Run the lower-link-inject pipeline with the multi-worker + partition + check_frame stages enabled,
/// returning the contract inputs `emit()` consumes. Mirrors mp-tcp-bufsync's `build_per_worker_partitioned`.
fn build_per_worker_partitioned(
    algo_src: &str,
    sched_src: &str,
) -> (
    std::collections::BTreeMap<compiler::event::WorkerId, Vec<compiler::event::Event>>,
    NameTables,
    compiler::sidecar::NameSidecar,
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

/// Write a kernels stub the `emit()` call reads + copies into the
/// generated project. The contents are irrelevant to emit-string
/// pinning (we don't compile / run the project), but `emit()` requires
/// the path to exist.
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

// --------------------------------------------------------------------
// Test 1: on_violation = panic
// --------------------------------------------------------------------
//
// Pins the per-iteration Instant measurement + the inline panic
// dispatch template emitted in `render_worker_events`'s Event::Loop
// arm (multi_worker.rs:625-653). The Panic branch is INLINE (no shared
// helper), unlike Log + Count, so this test catches drift between the
// inline template and the pthreads-sync inline template in lockstep.
// --------------------------------------------------------------------

#[test]
fn multi_worker_panic_emit_pins_per_thread_panic_template() {
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
    let out = scratch_dir("multi_worker_panic_emit_pins_per_thread_panic_template");
    let kernels_rs = write_kernels_stub(&out);
    let res = emit(&per_worker, &names, &sidecar, &kernels_rs, &out)
        .expect("multi-worker emit with on_violation=panic must succeed");

    let main_src = fs::read_to_string(&res.main_rs).expect("read main.rs");

    // partition=workers projects the source loop onto BOTH w0 and w1;
    // each rendered Event::Loop carries the same check_frame. So the
    // emitted main.rs contains EXACTLY 2 Instant::now() sites
    // (one per compute worker; host has no check_frame).
    let start_count = main_src
        .matches("let _check_start = std::time::Instant::now();")
        .count();
    assert_eq!(
        start_count, 2,
        "expected 2 Instant::now() instrumentation points (one per \
         partitioned compute worker w0/w1); got {start_count}.\nmain.rs:\n{main_src}"
    );
    let elapsed_count = main_src
        .matches("let _check_elapsed = _check_start.elapsed().as_nanos();")
        .count();
    assert_eq!(
        elapsed_count, 2,
        "expected 2 elapsed-nanos reads (one per compute worker); \
         got {elapsed_count}.\nmain.rs:\n{main_src}"
    );

    // The Panic template is INLINE (no shared helper) — pin the
    // literal byte-shape that pthreads-async's render_worker_events
    // emits. Threshold is 5ms = 5_000_000 ns.
    let panic_count = main_src
        .matches("panic!(\"latency budget violated on `check loop n`")
        .count();
    assert_eq!(
        panic_count, 2,
        "expected 2 panic-message sites (per compute worker); got {panic_count}.\
         \nmain.rs:\n{main_src}"
    );
    assert!(
        main_src.contains("if _check_elapsed > 5000000_u128"),
        "panic guard must compare against the 5ms = 5000000 ns threshold \
         literal.\nmain.rs:\n{main_src}"
    );
    assert!(
        main_src.contains("max 5000000 ns"),
        "panic message must contain `max 5000000 ns` (the threshold literal).\
         \nmain.rs:\n{main_src}"
    );

    // Multi-worker Panic emit must NOT include the Log eprintln or
    // any Count substrate.
    assert!(
        !main_src.contains("eprintln!(\"warning: check loop"),
        "Panic emit must NOT include the Log template:\n{main_src}"
    );
    assert!(
        !main_src.contains("NUC_CHECK_COUNT_"),
        "Panic emit must NOT include any Count statics:\n{main_src}"
    );
    assert!(
        !main_src.contains("struct NucCheckCountReporter {"),
        "Panic emit must NOT include the Count reporter struct:\n{main_src}"
    );
}

// --------------------------------------------------------------------
// Test 2: on_violation = log
// --------------------------------------------------------------------
//
// Pins the per-iteration measurement + `emit_log_branch` template
// (the shared helper from pthreads-sync, called per worker in
// render_worker_events). Drift between pthreads-sync and pthreads-
// async would manifest as a different number of matches or a
// different format string here.
// --------------------------------------------------------------------

#[test]
fn multi_worker_log_emit_pins_per_thread_eprintln_template() {
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
    let out = scratch_dir("multi_worker_log_emit_pins_per_thread_eprintln_template");
    let kernels_rs = write_kernels_stub(&out);
    let res = emit(&per_worker, &names, &sidecar, &kernels_rs, &out)
        .expect("multi-worker emit with on_violation=log must succeed");

    let main_src = fs::read_to_string(&res.main_rs).expect("read main.rs");

    // The measurement prelude is shared with Panic — 2 sites.
    let start_count = main_src
        .matches("let _check_start = std::time::Instant::now();")
        .count();
    assert_eq!(
        start_count, 2,
        "expected 2 Instant::now() sites under partition=workers; \
         got {start_count}.\nmain.rs:\n{main_src}"
    );

    // emit_log_branch's template (shared helper) — partition=workers
    // projects onto w0 + w1, so EXACTLY 2 sites.
    let eprintln_count = main_src
        .matches(
            "eprintln!(\"warning: check loop `n` violated latency_max=5000000 ns: iteration took {} ns\", _check_elapsed);",
        )
        .count();
    assert_eq!(
        eprintln_count, 2,
        "expected 2 Log eprintln sites (one per compute worker); got \
         {eprintln_count}. The shared template via emit_log_branch (TASK-0222) \
         must produce the SAME template across all consumers and across all \
         three tier-1 backends.\nmain.rs:\n{main_src}"
    );

    // Log emit must NOT include Panic or Count substrate.
    assert!(
        !main_src.contains("panic!(\"latency budget violated"),
        "Log emit must NOT include the Panic template:\n{main_src}"
    );
    assert!(
        !main_src.contains("NUC_CHECK_COUNT_"),
        "Log emit must NOT include any Count statics:\n{main_src}"
    );
    assert!(
        !main_src.contains("struct NucCheckCountReporter {"),
        "Log emit must NOT include the Count reporter struct:\n{main_src}"
    );
}

// --------------------------------------------------------------------
// Test 3: on_violation = count
// --------------------------------------------------------------------
//
// Pins ALL THREE Count templates fired through the shared helpers:
//   - file-scope `static NUC_CHECK_COUNT_<ident>` (emit_count_static)
//   - file-scope `struct NucCheckCountReporter` + Drop impl
//     (emit_count_reporter_struct)
//   - host-thread Drop-guard local in fn main (emit_count_guard_local)
//   - per-worker `fetch_add` branch (emit_count_branch)
//
// Shared-memory model: ONE static + ONE struct + ONE guard local +
// N fetch_add sites (one per compute worker). Mirrors pthreads-sync's
// multi_worker_check_loop_count_emit_* test (line 894 of pthreads-
// sync/tests/multi_worker.rs).
// --------------------------------------------------------------------

#[test]
fn multi_worker_count_emit_pins_shared_static_guard_and_fetch_add() {
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
    let out = scratch_dir("multi_worker_count_emit_pins_shared_static_guard_and_fetch_add");
    let kernels_rs = write_kernels_stub(&out);
    let res = emit(&per_worker, &names, &sidecar, &kernels_rs, &out)
        .expect("multi-worker emit with on_violation=count must succeed");

    let main_src = fs::read_to_string(&res.main_rs).expect("read main.rs");

    // (a) EXACTLY ONE file-scope static (deduped by sanitized ident
    // — both compute workers' Event::Loop carry the same check_frame
    // under partition=workers, so collect_unique_count_check_frames
    // dedups to a single entry).
    let static_count = main_src
        .matches(
            "static NUC_CHECK_COUNT_n: std::sync::atomic::AtomicU64 = \
             std::sync::atomic::AtomicU64::new(0);",
        )
        .count();
    assert_eq!(
        static_count, 1,
        "expected exactly 1 shared static AtomicU64 (deduped by ident across \
         compute workers via collect_unique_count_check_frames); got \
         {static_count}.\nmain.rs:\n{main_src}"
    );

    // (b) EXACTLY ONE reporter struct definition (shared by all
    // statics — single Drop impl serves all check_frame counters).
    let struct_count = main_src.matches("struct NucCheckCountReporter {").count();
    assert_eq!(
        struct_count, 1,
        "expected exactly 1 NucCheckCountReporter struct definition; got \
         {struct_count}.\nmain.rs:\n{main_src}"
    );
    let drop_impl_count = main_src
        .matches("impl Drop for NucCheckCountReporter {")
        .count();
    assert_eq!(
        drop_impl_count, 1,
        "expected exactly 1 Drop impl for the reporter struct; got \
         {drop_impl_count}.\nmain.rs:\n{main_src}"
    );

    // (c) EXACTLY ONE Drop-guard local in fn main (host thread owns
    // the aggregated Drop summary; runs after every handle.join()).
    let guard_count = main_src
        .matches("let _nuc_check_reporter_n = NucCheckCountReporter {")
        .count();
    assert_eq!(
        guard_count, 1,
        "expected exactly 1 NucCheckCountReporter guard local in fn main \
         (host thread owns the Drop summary); got {guard_count}.\n\
         main.rs:\n{main_src}"
    );

    // The guard local must bind the matching static + carry the
    // user's loop_var name + the threshold in ns.
    assert!(
        main_src.contains("counter: &NUC_CHECK_COUNT_n,"),
        "guard local must bind the matching counter.\nmain.rs:\n{main_src}"
    );
    assert!(
        main_src.contains("loop_var: \"n\","),
        "guard local must carry the user's loop_var name.\nmain.rs:\n{main_src}"
    );
    assert!(
        main_src.contains("threshold_ns: 5000000,"),
        "guard local must carry the 5ms threshold in ns.\nmain.rs:\n{main_src}"
    );

    // (d) EXACTLY 2 fetch_add sites (one per compute worker; host has
    // no check_frame under partition=workers).
    let fetch_add_count = main_src
        .matches(
            "if _check_elapsed > 5000000_u128 { \
             NUC_CHECK_COUNT_n.fetch_add(1, std::sync::atomic::Ordering::Relaxed); }",
        )
        .count();
    assert_eq!(
        fetch_add_count, 2,
        "expected exactly 2 fetch_add branch sites (one per compute worker; \
         host excluded under partition=workers); got {fetch_add_count}.\n\
         main.rs:\n{main_src}"
    );

    // The measurement prelude must still be there per worker (the
    // shared helpers depend on _check_elapsed being in scope).
    let start_count = main_src
        .matches("let _check_start = std::time::Instant::now();")
        .count();
    assert_eq!(
        start_count, 2,
        "Count path must still emit Instant::now() measurement per worker; \
         got {start_count}.\nmain.rs:\n{main_src}"
    );

    // Count emit must NOT include Panic or Log dispatch (Count is
    // exclusive — the shared render_worker_events match-arm is
    // ViolationKind-exhaustive).
    assert!(
        !main_src.contains("panic!(\"latency budget violated"),
        "Count emit must NOT include the Panic template:\n{main_src}"
    );
    assert!(
        !main_src.contains("eprintln!(\"warning: check loop"),
        "Count emit must NOT include the Log template:\n{main_src}"
    );
}
