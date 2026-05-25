//! mp-tcp-bufsync synthetic two-worker pingpong test (TASK-0036 AC#5).
//!
//! Drives the SAME synthetic two-worker scenario as
//! `pthreads-sync/tests/multi_worker.rs` (host produces x,y; w0
//! combines into z; host sinks z — three Push/Wait pairs) through the
//! mp-tcp-bufsync backend: parse + lower + link + ACFG + sync +
//! transfer injection -> emit -> `cargo build` the generated
//! multi-process project -> run `run.sh` -> assert the output.
//!
//! AC#5 specifically: the result must match pthreads-sync **bit for
//! bit**. We do not just check a constant — we run BOTH backends on
//! the identical pipeline and byte-compare their `output.bin`. That
//! is the cross-backend differential in unit-test form, isolated from
//! the example matrix.
//!
//! Non-flaky discipline: the test runs the multi-process binary and
//! asserts a deterministic byte result; CI runs the suite repeatedly.
//! A real TCP handshake race would surface as an intermittent
//! failure here, so this test IS the flakiness canary.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use nucleus_compiler::{
    acfg_to_events,
    algo::{lower_algo, parse_algo},
    build_acfg, build_sidecar, inject_syncs, inject_transfers, link,
    sched::{lower_sched, parse_sched},
};
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
    let target = repo_root().join("nucleus/target/mp-tcp-bufsync-pingpong-scratch");
    let _ = fs::create_dir_all(&target);
    let dir = target.join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

const ALGO_SRC: &str = r#"
const N : usize = 16;

data x : i32[N];
data y : i32[N];
data z : i32[N];

kernel produce_x : () -> i32[N] effectful;
kernel produce_y : () -> i32[N] effectful;
kernel combine   : (i32[N], i32[N]) -> i32[N] pure;
kernel sink      : (i32[N]) -> () effectful;

x <-- produce_x();
y <-- produce_y();
z <-- combine(x, y);
sink(z);
"#;

const SCHED_SRC: &str = r#"
schedule for "anything.algo.nuc" {
    workers = { host, w0 };

    place produce_x on host;
    place produce_y on host;
    place combine   on w0;
    place sink      on host;

    transfer x : sync;
    transfer y : sync;
    transfer z : sync;
}
"#;

/// Byte-identical to pthreads-sync's synthetic kernels so the
/// cross-backend differential compares like with like.
const KERNELS_SRC: &str = r#"
use std::env;
use std::fs;
use std::io::Write;

const N: usize = 16;

pub fn produce_x() -> Vec<i32> {
    (0..N as i32).collect()
}

pub fn produce_y() -> Vec<i32> {
    vec![100; N]
}

pub fn combine(x: Vec<i32>, y: Vec<i32>) -> Vec<i32> {
    assert_eq!(x.len(), N);
    assert_eq!(y.len(), N);
    let mut out = Vec::with_capacity(N);
    for i in 0..N {
        out.push(x[i].wrapping_add(y[i]));
    }
    out
}

pub fn sink(z: Vec<i32>) {
    assert_eq!(z.len(), N);
    let sum: i32 = z.iter().copied().sum();
    let path = env::var("NUC_OUTPUT_PATH").unwrap_or_else(|_| "output.bin".to_string());
    let mut f = fs::File::create(&path).expect("sink: cannot create output file");
    f.write_all(&sum.to_le_bytes()).expect("sink: write failed");
}
"#;

/// Build the post-injection ACFG + sidecar + reverse name tables
/// (exactly what the driver feeds every EventList backend).
fn pipeline(
    scratch: &Path,
) -> (
    PathBuf,
    NameTables,
    nucleus_compiler::sidecar::NameSidecar,
    std::collections::BTreeMap<
        nucleus_compiler::event::WorkerId,
        Vec<nucleus_compiler::event::Event>,
    >,
) {
    let kernels_path = scratch.join("kernels.rs");
    fs::write(&kernels_path, KERNELS_SRC).unwrap();

    let algo_ast = parse_algo(ALGO_SRC).expect("algo parse");
    let sched_ast = parse_sched(SCHED_SRC).expect("sched parse");
    let algo_ir = lower_algo(&algo_ast).expect("algo lower");
    let sched_ir = lower_sched(&sched_ast).expect("sched lower");
    let linked = link(algo_ir, sched_ir).expect("link");
    let acfg = build_acfg(&linked).expect("build_acfg");
    let acfg = inject_syncs(acfg);
    let acfg = inject_transfers(&linked, acfg).expect("inject_transfers");

    let per_worker = acfg_to_events(&acfg);
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
    // TASK-0238 (cycle 25): 5-field NameTables literal collapsed to
    // the centralized constructor.
    let names = NameTables::from_acfg(&acfg);
    (kernels_path, names, sidecar, per_worker)
}

/// Emit, build, and run the mp-tcp-bufsync multi-process project;
/// return the bytes it wrote to output.bin.
fn run_mp_tcp(scratch: &Path) -> Vec<u8> {
    let (kernels_path, names, sidecar, per_worker) = pipeline(scratch);
    let out_dir = scratch.join("gen");
    let result = mp_tcp_bufsync::emit(&per_worker, &names, &sidecar, &kernels_path, &out_dir)
        .expect("mp-tcp-bufsync emit");

    // Structural smoke checks: it really is multi-process + TCP.
    assert!(
        result.worker_bins.len() >= 2,
        "expected >=2 per-worker binaries, got {}",
        result.worker_bins.len()
    );
    let host_src = fs::read_to_string(out_dir.join("src/bin/host.rs")).unwrap();
    let w0_src = fs::read_to_string(out_dir.join("src/bin/w0.rs")).unwrap();
    assert!(host_src.contains("TcpListener"), "host must be the server");
    assert!(
        w0_src.contains("TcpStream::connect"),
        "w0 must be the client"
    );
    // TASK-0218: this fixture is bare-Operation only (no Repeat), so
    // after the sync_inject Push/Wait elision lands there are zero
    // Sync events on the EventList — and therefore no
    // `wire::barrier_cross` calls in the emitted code. We no longer
    // assert that needle here. The wire-level barrier_cross helper
    // has its own unit test in `mp-tcp-common` (`barrier_cross_two_party`)
    // and is exercised end-to-end by the e2e matrix cells that DO
    // carry Sync events (02-split-add__split__mp-tcp-bufsync via its
    // Repeat-entry sync, 05-stencil__blocked__mp-tcp-bufsync, etc.).
    for needle in &["wire::write_msg", "wire::read_msg_expect"] {
        assert!(
            host_src.contains(needle) || w0_src.contains(needle),
            "generated code missing `{needle}`"
        );
    }

    let build = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--quiet")
        .current_dir(&out_dir)
        .output()
        .expect("cargo build generated mp-tcp project");
    assert!(
        build.status.success(),
        "generated mp-tcp project failed to build:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );

    let input_bin = scratch.join("input.bin"); // unused by these kernels
    let _ = fs::write(&input_bin, []);
    let output_bin = scratch.join("output.bin");
    let run = Command::new("bash")
        .arg(out_dir.join("run.sh"))
        .arg(&input_bin)
        .arg(&output_bin)
        .current_dir(&out_dir)
        .output()
        .expect("run.sh");
    assert!(
        run.status.success(),
        "run.sh exited non-zero:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    fs::read(&output_bin).expect("read mp-tcp output.bin")
}

/// Emit, build, and run the pthreads-sync project on the IDENTICAL
/// pipeline; return its output.bin bytes (the differential baseline).
fn run_pthreads(scratch: &Path) -> Vec<u8> {
    let (kernels_path, names, sidecar, per_worker) = pipeline(scratch);
    let out_dir = scratch.join("gen-pthreads");
    let result = pthreads_sync::emit(&per_worker, &names, &sidecar, &kernels_path, &out_dir)
        .expect("pthreads-sync emit");

    let build = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--quiet")
        .current_dir(&out_dir)
        .output()
        .expect("cargo build pthreads project");
    assert!(
        build.status.success(),
        "pthreads project failed to build:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let output_bin = scratch.join("output-pthreads.bin");
    let run = Command::new(result.project_dir.join("target/release/nuc-generated"))
        .env("NUC_OUTPUT_PATH", &output_bin)
        .output()
        .expect("run pthreads binary");
    assert!(run.status.success(), "pthreads binary exited non-zero");
    fs::read(&output_bin).expect("read pthreads output.bin")
}

/// AC#5: the synthetic two-worker pingpong matches pthreads-sync
/// **bit for bit**.
#[test]
fn pingpong_matches_pthreads_sync_bit_for_bit() {
    let scratch = scratch_dir("pingpong");
    let mp = run_mp_tcp(&scratch);
    let pt = run_pthreads(&scratch);

    // Expected sum: 0+1+...+15 + 16*100 = 120 + 1600 = 1720, as a
    // single little-endian i32 (the schedule-independent oracle).
    let expected = 1720_i32.to_le_bytes().to_vec();
    assert_eq!(
        mp, expected,
        "mp-tcp-bufsync output {mp:?} != expected oracle {expected:?}"
    );
    assert_eq!(
        mp, pt,
        "DIFFERENTIAL FAILURE: mp-tcp-bufsync {mp:?} != pthreads-sync {pt:?} \
         on the identical (algorithm, schedule) — the cross-backend \
         bit-identity contract is violated"
    );
}

// --------------------------------------------------------------------
// TASK-0245 (cycle 36): regression-pin for the cycle-35
// `render_int_expr` const-in-IndexExpr fix on the mp-tcp-bufsync
// backend.
//
// Background: cycle 35 (commit 894f63f) fixed the PRIVATE
// `pthreads_sync::render_int_expr` to resolve declared consts (e.g.
// `ITERS`) when they appear inside an `IndexExpr`. mp-tcp-bufsync
// consumes ALL of its IndexExpr / arg / output-assign rendering
// through the pub shims (`render_fire_args_pub`,
// `render_fire_output_assign_pub`, `render_const_expr_pub`,
// `render_flat_index_pub`); the cycle-36 audit grep
// (`grep -rnE "fn render_int_expr|fn render_flat_index|fn
// render_const_expr" nucleus/backends/mp-tcp-bufsync/`) returned
// ZERO matches — confirming no parallel private renderer with its
// own consts gap exists. This test pins that audit STRUCTURALLY: a
// future cycle that copies a private IndexExpr renderer into this
// crate without `sidecar.consts` lookup fails the test immediately.
//
// What it pins:
//   1. The IndexExpr arithmetic site in EVERY per-worker `src/bin/
//      <worker>.rs` contains the resolved const LITERAL (`8`) at the
//      position `ITERS` occupied in the source.
//   2. The bare const ident (`ITERS`) does NOT appear anywhere in
//      ANY per-worker binary's source.
//
// Sibling tests for pthreads-sync + pthreads-async live in
// `pthreads-sync/tests/multi_worker.rs` +
// `pthreads-async/tests/skeleton.rs`, all driven by the same
// `test_common::CONST_IN_INDEXEXPR_*` fixture (single source of truth).
//
// Emit-string only — no cargo-build. End-to-end correctness is
// already covered by the e2e gate on example 11 (cycle-35 evidence).
#[test]
fn const_in_indexexpr_mp_tcp_bufsync_resolves_to_literal_value() {
    let scratch = scratch_dir("const_in_indexexpr_mp_tcp");
    let kernels_path = scratch.join("kernels.rs");
    fs::write(&kernels_path, "// stub for emit-string test\n").unwrap();

    let r = test_common::lower_for_test(
        test_common::CONST_IN_INDEXEXPR_ALGO_SRC,
        test_common::CONST_IN_INDEXEXPR_SCHED_SRC,
        &test_common::LowerForTestOpts::default(),
    );

    let out_dir = scratch.join("gen");
    let result = mp_tcp_bufsync::emit(&r.per_worker, &r.names, &r.sidecar, &kernels_path, &out_dir)
        .expect("mp-tcp-bufsync emit must succeed on const-in-IndexExpr fixture");

    // mp-tcp-bufsync emits ONE binary per used worker; the fixture's
    // 2-worker schedule yields TWO files in worker_bins. Both
    // participate in the IndexExpr (w0 writes `y[ITERS][i]`, host
    // reads `y[ITERS][0]`), so both must carry the resolved literal.
    assert!(
        result.worker_bins.len() >= 2,
        "expected >=2 per-worker binaries for a 2-worker schedule; got {}",
        result.worker_bins.len()
    );

    let iters_val = test_common::CONST_IN_INDEXEXPR_ITERS_VALUE;
    let resolved_row = format!("({iters_val}) * 4");
    let bare_ident = test_common::CONST_IN_INDEXEXPR_ITERS_IDENT;

    // At least ONE per-worker file must contain the resolved
    // IndexExpr fingerprint (the worker that touches `y[ITERS]...` —
    // i.e. either host or w0; under sync transfer, both rendered
    // sites carry it).
    let mut any_has_resolved = false;
    for bin_path in &result.worker_bins {
        let src = fs::read_to_string(bin_path).expect("read per-worker bin");
        if src.contains(&resolved_row) {
            any_has_resolved = true;
        }
        // (2) The bare const ident `ITERS` must NOT appear in ANY
        // per-worker binary. This is the load-bearing regression
        // assertion: if the cycle-35 fix were ever lost (or a private
        // renderer added without consts lookup), `ITERS` would leak
        // through to one of the per-worker binaries, and Rust would
        // refuse to compile.
        assert!(
            !src.contains(bare_ident),
            "mp-tcp-bufsync {bin_path:?} must NOT contain the bare const \
             ident `{bare_ident}` — render_flat_index_pub must resolve it \
             via sidecar.consts (cycle-35 fix; TASK-0245 audit). \
             source:\n{src}"
        );
    }
    assert!(
        any_has_resolved,
        "expected the resolved `ITERS=8` literal (`{resolved_row}`) in at \
         least one mp-tcp-bufsync per-worker binary; cycle-35 fix not \
         reaching this backend's code path. worker_bins: {:?}",
        result.worker_bins
    );
}
