//! Codegen-string tests for `check loop V : latency_max=T` on the
//! tier-1 backends (TASK-0052.02 AC#1/AC#3 codegen arm).
//!
//! Asserts the rendered main.rs contains the expected
//! `std::time::Instant::now()` measurement + threshold comparison +
//! panic message embedding {loop_var, measured_ns, threshold_ns}.
//!
//! Why string-asserting and not compile-and-run: a string assert is
//! deterministic, takes ~10ms, and proves the codegen *shape* — the
//! property AC#1/AC#3 is really about. The downstream `cargo build` +
//! binary run is covered by the e2e harness when an example with a
//! `check loop` gets promoted into the required-cell matrix (filed as
//! a follow-up; not in tier-1 today).

use std::collections::BTreeMap;
use std::fs;

use nucleus_compiler::acfg_to_events;
use nucleus_compiler::algo::{lower_algo, parse_algo};
use nucleus_compiler::event::{Event, WorkerId};
use nucleus_compiler::sched::{lower_sched, parse_sched};
use nucleus_compiler::sidecar::NameSidecar;
use nucleus_compiler::{
    apply_block_transforms, apply_partition_workers, build_acfg, build_sidecar,
    inject_check_frames, inject_syncs, inject_transfers, link,
};
use pthreads_sync::NameTables;

fn build_per_worker_with_names(
    algo_src: &str,
    sched_src: &str,
) -> (BTreeMap<WorkerId, Vec<Event>>, NameTables, NameSidecar) {
    let algo_ast = parse_algo(algo_src).expect("algo parse");
    let algo_ir = lower_algo(&algo_ast).expect("algo lower");
    let sched_ast = parse_sched(sched_src).expect("sched parse");
    let sched_ir = lower_sched(&sched_ast).expect("sched lower");
    let linked = link(algo_ir, sched_ir).expect("link");
    let acfg = build_acfg(&linked).expect("acfg");
    let acfg = apply_block_transforms(&linked, acfg).expect("block-transform");
    let acfg = apply_partition_workers(&linked, acfg).expect("partition-workers");
    let acfg = inject_syncs(acfg).expect("inject_syncs");
    let acfg = inject_transfers(&linked, acfg).expect("inject_transfers");
    let per_worker = acfg_to_events(&acfg);
    let per_worker = inject_check_frames(per_worker, &linked.sched.checks, &acfg.name_iter_vars);
    let sidecar = build_sidecar(&linked, &acfg).expect("sidecar");
    // TASK-0238 (cycle 25): 5-field NameTables literal collapsed to
    // the centralized constructor.
    let names = NameTables::from_acfg(&acfg);
    (per_worker, names, sidecar)
}

/// Locate the workspace root for fixture lookup.
fn repo_root() -> std::path::PathBuf {
    let here = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(std::path::Path::parent)
        .expect("CARGO_MANIFEST_DIR has ancestors")
        .to_path_buf()
}

fn scratch_dir(name: &str) -> std::path::PathBuf {
    // TASK-0426.01: per-call-unique scratch dir via the shared helper
    // (created once, never removed) — kills the remove/create race class.
    let target = repo_root().join("nucleus/target/check-frame-scratch");
    test_common::unique_scratch_dir(&target, name)
}

// --------------------------------------------------------------------
// pthreads-sync codegen
// --------------------------------------------------------------------

#[test]
fn pthreads_sync_emit_includes_panic_instrumentation_on_check_loop() {
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
    // 1ms threshold; element-wise inc over N=4 will finish in << 1ms
    // wall-time on any reasonable host, so the run-time path is the
    // success path (no panic). The codegen STRING, which is what we
    // assert here, embeds the threshold literal regardless.
    let sched = "\
schedule for \"a.algo.nuc\" {
    workers = { host };
    place load_input on host;
    place save_output on host;
    place inc on host;
    check loop n : latency_max = 1ms;
}
";
    let (per_worker, names, sidecar) = build_per_worker_with_names(algo, sched);

    // Write a stub kernels.rs so emit() can read it.
    let out = scratch_dir("pthreads_panic_codegen");
    let kernels_rs = out.join("kernels_stub.rs");
    fs::write(
        &kernels_rs,
        "pub fn load_input() -> Vec<i32> { vec![0; 4] }\n\
         pub fn save_output(_c: Vec<i32>) {}\n\
         pub fn inc(x: i32) -> i32 { x + 1 }\n",
    )
    .expect("write kernels stub");

    let res = pthreads_sync::emit(&per_worker, &names, &sidecar, &kernels_rs, &out)
        .expect("emit must succeed for a valid schedule with a check loop");
    let main_src = fs::read_to_string(&res.main_rs).expect("read main.rs");

    // AC#1 (codegen arm): timing measurement at iter start.
    assert!(
        main_src.contains("let _check_start = std::time::Instant::now();"),
        "main.rs must contain Instant::now() inside the `n` loop body. \
         Got:\n{main_src}"
    );
    // AC#1: comparison + AC#3: message includes loop_var + measured + threshold.
    assert!(
        main_src.contains("let _check_elapsed = _check_start.elapsed().as_nanos();"),
        "main.rs must contain the elapsed-nanos read. Got:\n{main_src}"
    );
    assert!(
        main_src.contains("if _check_elapsed > 1000000_u128"),
        "main.rs must contain `if _check_elapsed > 1000000_u128`. Got:\n{main_src}"
    );
    // The panic message embeds:
    //   - the loop_var name ("n")
    //   - the measured ns (runtime, via {} formatter)
    //   - the threshold ns ("1000000")
    assert!(
        main_src.contains("panic!(\"latency budget violated on `check loop n`"),
        "panic message must name the loop_var. Got:\n{main_src}"
    );
    assert!(
        main_src.contains("max 1000000 ns"),
        "panic message must contain `max 1000000 ns` (the threshold literal). Got:\n{main_src}"
    );
}

#[test]
fn pthreads_sync_emit_unchanged_without_check_loop_directive() {
    // AC#2 (codegen arm): a schedule WITHOUT `check loop V` is
    // byte-identical to the pre-TASK-0052.02 baseline — no Instant
    // call appears in the rendered source. This is the determinism
    // contract carrying through: every existing e2e cell must keep its
    // bit-identical output.
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
    let (per_worker, names, sidecar) = build_per_worker_with_names(algo, sched);
    let out = scratch_dir("pthreads_no_check");
    let kernels_rs = out.join("kernels_stub.rs");
    fs::write(
        &kernels_rs,
        "pub fn load_input() -> Vec<i32> { vec![0; 4] }\n\
         pub fn save_output(_c: Vec<i32>) {}\n\
         pub fn inc(x: i32) -> i32 { x + 1 }\n",
    )
    .unwrap();
    let res = pthreads_sync::emit(&per_worker, &names, &sidecar, &kernels_rs, &out).unwrap();
    let main_src = fs::read_to_string(&res.main_rs).unwrap();
    assert!(
        !main_src.contains("std::time::Instant"),
        "schedule with NO `check loop` MUST NOT emit Instant::now(). \
         The bit-identical baseline contract requires the success-path \
         emitted bytes to be unchanged. Got:\n{main_src}"
    );
    assert!(
        !main_src.contains("latency budget violated"),
        "no check loop -> no panic message embedded. Got:\n{main_src}"
    );
}

#[test]
fn pthreads_sync_emit_threshold_unit_normalisation_to_ns() {
    // AC#1 (codegen arm + AC#1 of TASK-0052.01 carry): the unit
    // normalisation produces a NANOSECOND literal in the rendered
    // source. 10ms must become 10_000_000.
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
    for (unit_literal, expected_ns_literal) in [
        ("10ms", "10000000"),
        ("250us", "250000"),
        ("3s", "3000000000"),
        ("42ns", "42"),
    ] {
        let sched = format!(
            "\
schedule for \"a.algo.nuc\" {{
    workers = {{ host }};
    place load_input on host;
    place save_output on host;
    place inc on host;
    check loop n : latency_max = {unit_literal};
}}
"
        );
        let (per_worker, names, sidecar) = build_per_worker_with_names(algo, &sched);
        let out = scratch_dir(&format!(
            "pthreads_unit_{}",
            unit_literal.replace(['/', '\\'], "_")
        ));
        let kernels_rs = out.join("kernels_stub.rs");
        fs::write(
            &kernels_rs,
            "pub fn load_input() -> Vec<i32> { vec![0; 4] }\n\
             pub fn save_output(_c: Vec<i32>) {}\n\
             pub fn inc(x: i32) -> i32 { x + 1 }\n",
        )
        .unwrap();
        let res = pthreads_sync::emit(&per_worker, &names, &sidecar, &kernels_rs, &out).unwrap();
        let main_src = fs::read_to_string(&res.main_rs).unwrap();
        assert!(
            main_src.contains(&format!("if _check_elapsed > {expected_ns_literal}_u128")),
            "unit {unit_literal} must normalise to {expected_ns_literal}ns in the rendered \
             source. Got:\n{main_src}"
        );
    }
}

// --------------------------------------------------------------------
// Negative arm: rendered code embeds a literally-impossible tight
// threshold (1ns), runs, and the panic message contains the right
// numbers.
//
// We do NOT compile-and-run here (that is what the e2e harness does
// when example 14 lands in tier-1). But we DO assert the codegen
// string contains the inputs that a downstream `cargo build && run`
// would surface. The full compile-and-run negative arm is a follow-up;
// see TASK-0052.04 notes.
// --------------------------------------------------------------------

#[test]
fn negative_tight_threshold_codegen_embeds_inputs_for_a_runnable_repro() {
    // 1ns is structurally allowed (>0 per TASK-0052.01) and is below
    // any conceivable wall-clock cost of a function call + atomic
    // load — so a generated binary would panic on iteration 0.
    let algo = "\
const N : usize = 1;
data a : i32[N];
data c : i32[N];
kernel load_input  : ()      -> i32[N] effectful;
kernel save_output : (i32[N]) -> () effectful;
kernel slow : (i32) -> i32 pure;
a <-- load_input();
for n : 0 .. N {
    c[n] <-- slow(a[n]);
}
save_output(c);
";
    let sched = "\
schedule for \"a.algo.nuc\" {
    workers = { host };
    place load_input on host;
    place save_output on host;
    place slow on host;
    check loop n : latency_max = 1ns;
}
";
    let (per_worker, names, sidecar) = build_per_worker_with_names(algo, sched);
    let out = scratch_dir("pthreads_negative_1ns");
    let kernels_rs = out.join("kernels_stub.rs");
    fs::write(
        &kernels_rs,
        "pub fn load_input() -> Vec<i32> { vec![0; 1] }\n\
         pub fn save_output(_c: Vec<i32>) {}\n\
         pub fn slow(x: i32) -> i32 { std::thread::sleep(std::time::Duration::from_millis(1)); x }\n",
    )
    .unwrap();
    let res = pthreads_sync::emit(&per_worker, &names, &sidecar, &kernels_rs, &out).unwrap();
    let main_src = fs::read_to_string(&res.main_rs).unwrap();
    assert!(
        main_src.contains("if _check_elapsed > 1_u128"),
        "1ns must render as the literal `1_u128`. Got:\n{main_src}"
    );
    assert!(
        main_src.contains("max 1 ns"),
        "panic message must name the (tight) threshold literally. Got:\n{main_src}"
    );
}

// --------------------------------------------------------------------
// mp-tcp-bufsync codegen — only meaningful on a multi-process
// schedule because the file uses Event::Sync. A single-worker
// schedule emits via pthreads_sync internals (so test in pthreads
// above is the right place for single-worker). Skipping a dedicated
// mp-tcp test for now; the multi-worker mp-tcp branch with a
// `check loop` is exercised when example 14 (multi-MCU) lands in
// tier-1 — a follow-up.
// --------------------------------------------------------------------

// --------------------------------------------------------------------
// AC#3 (end-to-end): compile-and-run positive + negative.
//
// `cargo` is available inside `nix develop`. These tests cargo-build
// the rendered project and run the resulting binary; AC#3 demands the
// negative (tight threshold) actually panic with a clear message and
// the positive (generous threshold) run clean. Two tests, each ~10s
// (cargo init + small build); run under `just test` and gate before
// commit.
// --------------------------------------------------------------------

use std::process::Command;

fn cargo_build_and_run(project_dir: &std::path::Path) -> std::process::Output {
    // Per-project target dir so concurrent tests do NOT race on the
    // same `nuc-generated` binary (cargo test parallelises). Naming
    // by project_dir basename keeps the dir deterministic and
    // human-inspectable across runs.
    let leaf = project_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("default");
    let target_dir = repo_root()
        .join("nucleus/target/check-frame-binaries")
        .join(leaf);
    let build = Command::new("cargo")
        .arg("build")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(project_dir.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(&target_dir)
        .output()
        .expect("cargo build");
    if !build.status.success() {
        panic!(
            "cargo build of generated project failed:\nstdout:{}\nstderr:{}",
            String::from_utf8_lossy(&build.stdout),
            String::from_utf8_lossy(&build.stderr)
        );
    }
    let bin = target_dir.join("debug/nuc-generated");
    assert!(
        bin.exists(),
        "generated binary missing at {}",
        bin.display()
    );
    Command::new(&bin).output().expect("run generated binary")
}

#[test]
fn ac3_positive_generous_threshold_runs_clean() {
    // 1s threshold; the kernel is trivial — the loop iteration cannot
    // exceed 1 second wall-clock on any reasonable machine. The
    // generated binary must exit 0 and produce no panic message.
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
    check loop n : latency_max = 1s;
}
";
    let (per_worker, names, sidecar) = build_per_worker_with_names(algo, sched);
    let out = scratch_dir("ac3_positive");
    let kernels_rs = out.join("kernels.rs");
    fs::write(
        &kernels_rs,
        "pub fn load_input() -> Vec<i32> { vec![1, 2, 3, 4] }\n\
         pub fn save_output(c: Vec<i32>) { println!(\"{:?}\", c); }\n\
         pub fn inc(x: i32) -> i32 { x + 1 }\n",
    )
    .unwrap();
    pthreads_sync::emit(&per_worker, &names, &sidecar, &kernels_rs, &out).unwrap();
    let out = cargo_build_and_run(&out);
    assert!(
        out.status.success(),
        "AC#3 positive: generous 1s threshold must run clean. \
         exit={:?}\nstdout:{}\nstderr:{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        !String::from_utf8_lossy(&out.stderr).contains("latency budget violated"),
        "AC#3 positive: stderr must contain no panic message"
    );
}

#[test]
fn ac3_negative_tight_threshold_panics_with_loop_var_and_numbers() {
    // 1ns threshold; the inc kernel adds at least a function call,
    // which dwarfs 1ns on any host. Binary must exit non-zero with a
    // panic message naming the loop_var + measured ns + threshold ns.
    let algo = "\
const N : usize = 1;
data a : i32[N];
data c : i32[N];
kernel load_input  : ()      -> i32[N] effectful;
kernel save_output : (i32[N]) -> () effectful;
kernel slow : (i32) -> i32 pure;
a <-- load_input();
for n : 0 .. N {
    c[n] <-- slow(a[n]);
}
save_output(c);
";
    let sched = "\
schedule for \"a.algo.nuc\" {
    workers = { host };
    place load_input on host;
    place save_output on host;
    place slow on host;
    check loop n : latency_max = 1ns;
}
";
    let (per_worker, names, sidecar) = build_per_worker_with_names(algo, sched);
    let out = scratch_dir("ac3_negative");
    let kernels_rs = out.join("kernels.rs");
    fs::write(
        &kernels_rs,
        // Deliberately slow: sleep 1ms so the elapsed-ns measurement
        // CANNOT be 1ns regardless of host noise.
        "pub fn load_input() -> Vec<i32> { vec![0] }\n\
         pub fn save_output(_c: Vec<i32>) {}\n\
         pub fn slow(x: i32) -> i32 { std::thread::sleep(std::time::Duration::from_millis(1)); x }\n",
    )
    .unwrap();
    pthreads_sync::emit(&per_worker, &names, &sidecar, &kernels_rs, &out).unwrap();
    let out = cargo_build_and_run(&out);
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    // Rust panic exits with code 101.
    assert_eq!(
        out.status.code(),
        Some(101),
        "AC#3 negative: tight threshold must terminate with rustc \
         panic exit code 101. exit={:?}\nstderr:{}",
        out.status.code(),
        stderr
    );
    // Message includes:
    //   1. the user's loop_var name ("n")
    //   2. the measured ns (some number > 1)
    //   3. the threshold ns (1)
    assert!(
        stderr.contains("latency budget violated on `check loop n`"),
        "panic message must name the loop_var. stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("max 1 ns"),
        "panic message must contain `max 1 ns` (threshold). stderr:\n{stderr}"
    );
    // The measured-ns value is dynamic but MUST be greater than the
    // 1ns threshold for the panic to have fired. Extract it.
    // Message form: "iteration took {N} ns, max 1 ns"
    let took = stderr
        .split("iteration took ")
        .nth(1)
        .and_then(|s| s.split(" ns").next())
        .and_then(|s| s.parse::<u128>().ok())
        .unwrap_or_else(|| panic!("could not parse measured ns from: {stderr}"));
    assert!(took > 1, "measured ns ({took}) must exceed threshold (1)");
}

// --------------------------------------------------------------------
// TASK-0052.04: on_violation=log + on_violation=count codegen tests.
//
// Two backends share the codegen helpers (collect_count_check_frames,
// emit_count_reporter_struct, sanitize_loop_var) from pthreads-sync;
// the mp-tcp sibling test file pins the SAME emit strings on the
// mp-tcp side. A drift between backends will fail one of the two
// suites loudly.
// --------------------------------------------------------------------

#[test]
fn pthreads_sync_emit_includes_eprintln_on_log_violation() {
    // AC#1 (codegen arm): on_violation=log emits eprintln, NOT panic.
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
    let (per_worker, names, sidecar) = build_per_worker_with_names(algo, sched);
    let out = scratch_dir("pthreads_log_codegen");
    let kernels_rs = out.join("kernels_stub.rs");
    fs::write(
        &kernels_rs,
        "pub fn load_input() -> Vec<i32> { vec![0; 4] }\n\
         pub fn save_output(_c: Vec<i32>) {}\n\
         pub fn inc(x: i32) -> i32 { x + 1 }\n",
    )
    .unwrap();
    let res = pthreads_sync::emit(&per_worker, &names, &sidecar, &kernels_rs, &out).unwrap();
    let main_src = fs::read_to_string(&res.main_rs).unwrap();
    // Measurement at iter start (shared with Panic path).
    assert!(
        main_src.contains("let _check_start = std::time::Instant::now();"),
        "Log path must still emit Instant measurement. Got:\n{main_src}"
    );
    // eprintln, NOT panic.
    assert!(
        main_src.contains("eprintln!(\"warning: check loop `n` violated latency_max=1000000 ns:"),
        "Log path must emit the eprintln warning. Got:\n{main_src}"
    );
    assert!(
        !main_src.contains("panic!(\"latency budget violated"),
        "Log path must NOT emit panic!. Got:\n{main_src}"
    );
    // No file-scope Count machinery either.
    assert!(
        !main_src.contains("NUC_CHECK_COUNT_"),
        "Log-only schedule must not emit any Count statics. Got:\n{main_src}"
    );
    assert!(
        !main_src.contains("NucCheckCountReporter"),
        "Log-only schedule must not emit the Count reporter struct. Got:\n{main_src}"
    );
}

#[test]
fn pthreads_sync_emit_includes_atomic_and_reporter_on_count_violation() {
    // AC#2 (codegen arm): on_violation=count emits a file-scope
    // AtomicU64 static + a Drop-guard struct + a guard local in main.
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
    let (per_worker, names, sidecar) = build_per_worker_with_names(algo, sched);
    let out = scratch_dir("pthreads_count_codegen");
    let kernels_rs = out.join("kernels_stub.rs");
    fs::write(
        &kernels_rs,
        "pub fn load_input() -> Vec<i32> { vec![0; 4] }\n\
         pub fn save_output(_c: Vec<i32>) {}\n\
         pub fn inc(x: i32) -> i32 { x + 1 }\n",
    )
    .unwrap();
    let res = pthreads_sync::emit(&per_worker, &names, &sidecar, &kernels_rs, &out).unwrap();
    let main_src = fs::read_to_string(&res.main_rs).unwrap();
    // File-scope: static counter + reporter struct + Drop impl.
    assert!(
        main_src.contains(
            "static NUC_CHECK_COUNT_n: std::sync::atomic::AtomicU64 = \
             std::sync::atomic::AtomicU64::new(0);"
        ),
        "Count path must emit a per-loop AtomicU64 static. Got:\n{main_src}"
    );
    assert!(
        main_src.contains("struct NucCheckCountReporter {"),
        "Count path must emit the reporter struct. Got:\n{main_src}"
    );
    assert!(
        main_src.contains("impl Drop for NucCheckCountReporter {"),
        "Count path must emit the Drop impl. Got:\n{main_src}"
    );
    // The Drop body's summary line embeds threshold and count.
    assert!(
        main_src
            .contains("eprintln!(\"check loop `{}` violated latency_max={} ns: {} occurrence(s)\""),
        "Drop body must print the summary line. Got:\n{main_src}"
    );
    // In-main: the guard local + the fetch_add inside the loop body.
    assert!(
        main_src.contains("let _nuc_check_reporter_n = NucCheckCountReporter {"),
        "main must instantiate the per-loop guard local. Got:\n{main_src}"
    );
    assert!(
        main_src.contains("counter: &NUC_CHECK_COUNT_n,"),
        "guard local must bind the matching counter. Got:\n{main_src}"
    );
    assert!(
        main_src.contains("loop_var: \"n\","),
        "guard local must carry the user's loop_var name. Got:\n{main_src}"
    );
    assert!(
        main_src.contains("threshold_ns: 1000000,"),
        "guard local must carry the threshold in ns. Got:\n{main_src}"
    );
    assert!(
        main_src.contains(
            "if _check_elapsed > 1000000_u128 { \
             NUC_CHECK_COUNT_n.fetch_add(1, std::sync::atomic::Ordering::Relaxed); }"
        ),
        "loop body must fetch_add on threshold violation. Got:\n{main_src}"
    );
    // No panic, no eprintln-on-each-iter (Count is summary-at-end).
    assert!(
        !main_src.contains("panic!(\"latency budget violated"),
        "Count path must NOT emit panic!. Got:\n{main_src}"
    );
    assert!(
        !main_src.contains("eprintln!(\"warning: check loop `n` violated"),
        "Count path must NOT emit the Log-per-violation eprintln. Got:\n{main_src}"
    );
}

#[test]
fn pthreads_sync_emit_count_emits_one_static_per_check_loop() {
    // Two distinct check loops, both Count. Each must get its own
    // static + its own guard local; the reporter struct is emitted ONCE
    // (single Drop impl serves both statics).
    let algo = "\
const N : usize = 4;
const M : usize = 4;
data a : i32[N];
data b : i32[M];
data c : i32[N];
data d : i32[M];
kernel load_a : () -> i32[N] effectful;
kernel load_b : () -> i32[M] effectful;
kernel save_c : (i32[N]) -> () effectful;
kernel save_d : (i32[M]) -> () effectful;
kernel inc : (i32) -> i32 pure;
kernel dec : (i32) -> i32 pure;
a <-- load_a();
b <-- load_b();
for n : 0 .. N {
    c[n] <-- inc(a[n]);
}
for m : 0 .. M {
    d[m] <-- dec(b[m]);
}
save_c(c);
save_d(d);
";
    let sched = "\
schedule for \"a.algo.nuc\" {
    workers = { host };
    place load_a on host;
    place load_b on host;
    place save_c on host;
    place save_d on host;
    place inc on host;
    place dec on host;
    check loop n : latency_max = 1ms, on_violation = count;
    check loop m : latency_max = 2ms, on_violation = count;
}
";
    let (per_worker, names, sidecar) = build_per_worker_with_names(algo, sched);
    let out = scratch_dir("pthreads_count_two_loops");
    let kernels_rs = out.join("kernels_stub.rs");
    fs::write(
        &kernels_rs,
        "pub fn load_a() -> Vec<i32> { vec![0; 4] }\n\
         pub fn load_b() -> Vec<i32> { vec![0; 4] }\n\
         pub fn save_c(_c: Vec<i32>) {}\n\
         pub fn save_d(_d: Vec<i32>) {}\n\
         pub fn inc(x: i32) -> i32 { x + 1 }\n\
         pub fn dec(x: i32) -> i32 { x - 1 }\n",
    )
    .unwrap();
    let res = pthreads_sync::emit(&per_worker, &names, &sidecar, &kernels_rs, &out).unwrap();
    let main_src = fs::read_to_string(&res.main_rs).unwrap();
    // Two distinct statics.
    assert!(
        main_src.contains("static NUC_CHECK_COUNT_n: std::sync::atomic::AtomicU64"),
        "must emit the `n` static. Got:\n{main_src}"
    );
    assert!(
        main_src.contains("static NUC_CHECK_COUNT_m: std::sync::atomic::AtomicU64"),
        "must emit the `m` static. Got:\n{main_src}"
    );
    // Exactly ONE reporter struct (single Drop impl serves both).
    let struct_count = main_src.matches("struct NucCheckCountReporter {").count();
    assert_eq!(
        struct_count, 1,
        "must emit exactly one NucCheckCountReporter struct (got {struct_count}); \
         the Drop impl is reused. Source:\n{main_src}"
    );
    let drop_count = main_src
        .matches("impl Drop for NucCheckCountReporter {")
        .count();
    assert_eq!(
        drop_count, 1,
        "must emit exactly one Drop impl (got {drop_count}). Source:\n{main_src}"
    );
    // Two guard locals.
    assert!(
        main_src.contains("let _nuc_check_reporter_n ="),
        "must emit `n` guard local. Got:\n{main_src}"
    );
    assert!(
        main_src.contains("let _nuc_check_reporter_m ="),
        "must emit `m` guard local. Got:\n{main_src}"
    );
}

// --------------------------------------------------------------------
// AC#3 end-to-end: compile-and-run for Log and Count.
//
// The Log/Count handlers are positive-path; they must run cleanly to
// completion regardless of whether the threshold was violated. We
// build TIGHT-threshold variants (1ns) deliberately so the violation
// arm fires — the test asserts exit 0 (NOT panic) and the right
// stderr shape. The stdout is asserted unchanged (determinism on the
// differential channel).
// --------------------------------------------------------------------

#[test]
fn ac3_log_tight_threshold_runs_to_completion_and_logs() {
    let algo = "\
const N : usize = 2;
data a : i32[N];
data c : i32[N];
kernel load_input  : ()      -> i32[N] effectful;
kernel save_output : (i32[N]) -> () effectful;
kernel slow : (i32) -> i32 pure;
a <-- load_input();
for n : 0 .. N {
    c[n] <-- slow(a[n]);
}
save_output(c);
";
    let sched = "\
schedule for \"a.algo.nuc\" {
    workers = { host };
    place load_input on host;
    place save_output on host;
    place slow on host;
    check loop n : latency_max = 1ns, on_violation = log;
}
";
    let (per_worker, names, sidecar) = build_per_worker_with_names(algo, sched);
    let out = scratch_dir("ac3_log_tight");
    let kernels_rs = out.join("kernels.rs");
    fs::write(
        &kernels_rs,
        "pub fn load_input() -> Vec<i32> { vec![1, 2] }\n\
         pub fn save_output(c: Vec<i32>) { println!(\"{:?}\", c); }\n\
         pub fn slow(x: i32) -> i32 { std::thread::sleep(std::time::Duration::from_millis(1)); x + 1 }\n",
    )
    .unwrap();
    pthreads_sync::emit(&per_worker, &names, &sidecar, &kernels_rs, &out).unwrap();
    let out = cargo_build_and_run(&out);
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    // Exit 0: Log handler does NOT terminate.
    assert!(
        out.status.success(),
        "AC#3 Log: must exit 0 even when threshold violated. exit={:?}\nstderr:{stderr}\nstdout:{stdout}",
        out.status.code()
    );
    // Stderr contains at least one warning line, with loop_var + threshold.
    assert!(
        stderr.contains("warning: check loop `n` violated latency_max=1 ns"),
        "AC#3 Log: stderr must contain the eprintln warning. stderr:\n{stderr}"
    );
    // Stdout contains the kernel output (println from save_output) —
    // the cross-backend differential reads stdout, so Log handler must
    // not perturb it.
    assert!(
        stdout.contains("[2, 3]"),
        "AC#3 Log: stdout must contain the kernel-produced output unchanged. stdout:\n{stdout}"
    );
}

#[test]
fn ac3_count_tight_threshold_runs_and_prints_summary_at_exit() {
    let algo = "\
const N : usize = 3;
data a : i32[N];
data c : i32[N];
kernel load_input  : ()      -> i32[N] effectful;
kernel save_output : (i32[N]) -> () effectful;
kernel slow : (i32) -> i32 pure;
a <-- load_input();
for n : 0 .. N {
    c[n] <-- slow(a[n]);
}
save_output(c);
";
    let sched = "\
schedule for \"a.algo.nuc\" {
    workers = { host };
    place load_input on host;
    place save_output on host;
    place slow on host;
    check loop n : latency_max = 1ns, on_violation = count;
}
";
    let (per_worker, names, sidecar) = build_per_worker_with_names(algo, sched);
    let out = scratch_dir("ac3_count_tight");
    let kernels_rs = out.join("kernels.rs");
    fs::write(
        &kernels_rs,
        "pub fn load_input() -> Vec<i32> { vec![1, 2, 3] }\n\
         pub fn save_output(c: Vec<i32>) { println!(\"{:?}\", c); }\n\
         pub fn slow(x: i32) -> i32 { std::thread::sleep(std::time::Duration::from_millis(1)); x + 1 }\n",
    )
    .unwrap();
    pthreads_sync::emit(&per_worker, &names, &sidecar, &kernels_rs, &out).unwrap();
    let out = cargo_build_and_run(&out);
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        out.status.success(),
        "AC#3 Count: must exit 0. exit={:?}\nstderr:{stderr}\nstdout:{stdout}",
        out.status.code()
    );
    // Summary appears EXACTLY once (Drop fires once at fn-main return).
    let summary_line = "check loop `n` violated latency_max=1 ns:";
    let n_occurrences = stderr.matches(summary_line).count();
    assert_eq!(
        n_occurrences, 1,
        "AC#3 Count: summary must appear exactly once on stderr (got {n_occurrences}). stderr:\n{stderr}"
    );
    // Parse the occurrence count from "... 1 ns: {N} occurrence(s)".
    let count_part = stderr
        .split(summary_line)
        .nth(1)
        .and_then(|tail| tail.trim().split(' ').next())
        .and_then(|n| n.parse::<u64>().ok())
        .unwrap_or_else(|| panic!("could not parse occurrence count from: {stderr}"));
    // The slow kernel sleeps 1ms each call — every one of N=3 iters
    // exceeds 1ns. So the count is exactly N. The assertion is `>= 1`
    // not `== 3` to stay robust against host noise that might shorten
    // (unlikely) one of the sleeps (it can't go below 1ns, but be
    // belt-and-braces — the assertion of value is that >0 violations
    // were counted, not the exact number).
    assert!(
        count_part >= 1,
        "AC#3 Count: must count at least 1 violation; got {count_part}. stderr:\n{stderr}"
    );
    // Stdout unchanged by Count handler (the differential channel).
    assert!(
        stdout.contains("[2, 3, 4]"),
        "AC#3 Count: stdout must contain the kernel-produced output. stdout:\n{stdout}"
    );
}

#[test]
fn ac3_count_no_violation_emits_no_summary() {
    // Generous threshold (1s); the kernel is trivial. The Drop guard
    // SHOULD NOT print anything on a clean run (the `n > 0` gate).
    // This is the determinism-on-stderr-too property for clean runs.
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
    check loop n : latency_max = 1s, on_violation = count;
}
";
    let (per_worker, names, sidecar) = build_per_worker_with_names(algo, sched);
    let out = scratch_dir("ac3_count_generous");
    let kernels_rs = out.join("kernels.rs");
    fs::write(
        &kernels_rs,
        "pub fn load_input() -> Vec<i32> { vec![1, 2, 3, 4] }\n\
         pub fn save_output(c: Vec<i32>) { println!(\"{:?}\", c); }\n\
         pub fn inc(x: i32) -> i32 { x + 1 }\n",
    )
    .unwrap();
    pthreads_sync::emit(&per_worker, &names, &sidecar, &kernels_rs, &out).unwrap();
    let out = cargo_build_and_run(&out);
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(out.status.success(), "must exit 0. stderr:\n{stderr}");
    assert!(
        !stderr.contains("check loop `n` violated"),
        "AC#3 Count: clean run MUST NOT print a summary (n==0 gate). stderr:\n{stderr}"
    );
}
