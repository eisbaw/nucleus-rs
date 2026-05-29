//! mp-tcp-poll synthetic two-worker pingpong test (TASK-0044.02.02
//! cycle 195, sibling of mp-tcp-bufsync's `tests/pingpong.rs`).
//!
//! Drives the SAME synthetic two-worker scenario (host produces x,y;
//! w0 combines into z; host sinks z — three Push/Wait pairs) through
//! the mp-tcp-poll backend: parse + lower + link + ACFG + sync +
//! transfer injection → emit → cargo build → run.sh → assert.
//!
//! Cross-backend differential: the result must match pthreads-sync
//! AND mp-tcp-bufsync **bit for bit** — the same algorithm + schedule
//! ought to produce byte-identical output across every backend that
//! supports the schedule's capability surface.
//!
//! Also pins the const-in-IndexExpr forward-carry (cycle 193): a
//! multi-worker schedule using `test_common::CONST_IN_INDEXEXPR_*`
//! must resolve `ITERS` to the literal value `8` in every per-worker
//! binary. The byte-identical-vs-bufsync test ALONE does NOT bite
//! (both backends could drift in lockstep through
//! `pthreads_sync::render_const_expr_pub`); this independent pin is
//! the regression guard.

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
        .expect("three ancestors above mp-tcp-poll crate")
        .to_path_buf()
}

fn scratch_dir(name: &str) -> PathBuf {
    let target = repo_root().join("nucleus/target/mp-tcp-poll-pingpong-scratch");
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

/// Byte-identical to mp-tcp-bufsync's pingpong kernels — same
/// cross-backend differential input.
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
    let acfg = inject_syncs(acfg).expect("inject_syncs");
    let acfg = inject_transfers(&linked, acfg).expect("inject_transfers");

    let per_worker = acfg_to_events(&acfg);
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
    let names = NameTables::from_acfg(&acfg);
    (kernels_path, names, sidecar, per_worker)
}

/// Emit, build, and run the mp-tcp-poll multi-process project;
/// return the bytes it wrote to output.bin.
fn run_mp_tcp_poll(scratch: &Path) -> Vec<u8> {
    let (kernels_path, names, sidecar, per_worker) = pipeline(scratch);
    let out_dir = scratch.join("gen-poll");
    let result = mp_tcp_poll::emit(&per_worker, &names, &sidecar, &kernels_path, &out_dir)
        .expect("mp-tcp-poll emit");

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
    // Cycle-195 needles (the poll-variant swap). Bufsync's pingpong
    // test elides the barrier assertion because the fixture is
    // bare-Operation only; mp-tcp-poll inherits the same elision.
    for needle in &["wire::write_msg_poll", "wire::read_msg_expect_poll"] {
        assert!(
            host_src.contains(needle) || w0_src.contains(needle),
            "generated mp-tcp-poll code missing `{needle}` — cycle-195 swap regressed"
        );
    }
    // apply_nonblocking must appear in every per-worker bin (sock-setup
    // pairs with apply_sock_buf). Silent-sibling guard.
    assert!(
        host_src.contains("wire::apply_nonblocking"),
        "host bin missing wire::apply_nonblocking — silent-sibling regression"
    );
    assert!(
        w0_src.contains("wire::apply_nonblocking"),
        "w0 bin missing wire::apply_nonblocking — silent-sibling regression"
    );

    let build = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--quiet")
        .current_dir(&out_dir)
        .output()
        .expect("cargo build generated mp-tcp-poll project");
    assert!(
        build.status.success(),
        "generated mp-tcp-poll project failed to build:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );

    let input_bin = scratch.join("input.bin");
    let _ = fs::write(&input_bin, []);
    let output_bin = scratch.join("output-poll.bin");
    let run = Command::new("bash")
        .arg(out_dir.join("run.sh"))
        .arg(&input_bin)
        .arg(&output_bin)
        .current_dir(&out_dir)
        .output()
        .expect("run.sh");
    assert!(
        run.status.success(),
        "mp-tcp-poll run.sh exited non-zero:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    fs::read(&output_bin).expect("read mp-tcp-poll output.bin")
}

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

/// AC#2 of TASK-0044.02.02 (synthetic): the synthetic two-worker
/// pingpong matches pthreads-sync **bit for bit**. The bigger
/// in-tree e2e matrix carries the example-level bit-identicality on
/// the 8 promoted cells (02-split, 03-reduction/distributed, etc.).
#[test]
fn pingpong_matches_pthreads_sync_bit_for_bit() {
    let scratch = scratch_dir("pingpong");
    let mp = run_mp_tcp_poll(&scratch);
    let pt = run_pthreads(&scratch);

    let expected = 1720_i32.to_le_bytes().to_vec();
    assert_eq!(
        mp, expected,
        "mp-tcp-poll output {mp:?} != expected oracle {expected:?}"
    );
    assert_eq!(
        mp, pt,
        "DIFFERENTIAL FAILURE: mp-tcp-poll {mp:?} != pthreads-sync {pt:?} \
         on the identical (algorithm, schedule) — the cross-backend \
         bit-identity contract is violated"
    );
}

// --------------------------------------------------------------------
// Forward-carry from TASK-0044.01.02 cycle 193: const-in-IndexExpr
// independent regression pin. Mirrors mp-tcp-bufsync's
// `const_in_indexexpr_mp_tcp_bufsync_resolves_to_literal_value`.
//
// Why the pin is needed when multi-worker lands but NOT for single-
// worker: single-worker delegation is defended by the byte-identical-
// vs-mp-tcp-bufsync test (tests/single_worker_emit.rs); multi-worker
// independent codegen needs its own const-in-IndexExpr pin to catch
// regressions of `pthreads_sync::render_const_expr_pub` that wouldn't
// be caught by a byte-identical comparison (both backends could drift
// in lockstep). The TASK-0044.02.02 brief calls this out explicitly.
// --------------------------------------------------------------------

#[test]
fn const_in_indexexpr_mp_tcp_poll_resolves_to_literal_value() {
    let scratch = scratch_dir("const_in_indexexpr_mp_tcp_poll");
    let kernels_path = scratch.join("kernels.rs");
    fs::write(&kernels_path, "// stub for emit-string test\n").unwrap();

    let r = test_common::lower_for_test(
        test_common::CONST_IN_INDEXEXPR_ALGO_SRC,
        test_common::CONST_IN_INDEXEXPR_SCHED_SRC,
        &test_common::LowerForTestOpts::default(),
    );

    let out_dir = scratch.join("gen");
    let result = mp_tcp_poll::emit(&r.per_worker, &r.names, &r.sidecar, &kernels_path, &out_dir)
        .expect("mp-tcp-poll emit must succeed on const-in-IndexExpr fixture");

    assert!(
        result.worker_bins.len() >= 2,
        "expected >=2 per-worker binaries for a 2-worker schedule; got {}",
        result.worker_bins.len()
    );

    let iters_val = test_common::CONST_IN_INDEXEXPR_ITERS_VALUE;
    let resolved_row = format!("({iters_val}) * 4");
    let bare_ident = test_common::CONST_IN_INDEXEXPR_ITERS_IDENT;

    let mut any_has_resolved = false;
    for bin_path in &result.worker_bins {
        let src = fs::read_to_string(bin_path).expect("read per-worker bin");
        if src.contains(&resolved_row) {
            any_has_resolved = true;
        }
        assert!(
            !src.contains(bare_ident),
            "mp-tcp-poll {bin_path:?} must NOT contain the bare const \
             ident `{bare_ident}` — render_flat_index_pub must resolve it \
             via sidecar.consts (cycle-35 fix; TASK-0245 audit; \
             cycle-193 forward-carry to TASK-0044.02.02). \
             source:\n{src}"
        );
    }
    assert!(
        any_has_resolved,
        "expected the resolved `ITERS=8` literal (`{resolved_row}`) in at \
         least one mp-tcp-poll per-worker binary; cycle-35 fix not reaching \
         mp-tcp-poll's multi-worker codegen. worker_bins: {:?}",
        result.worker_bins
    );
}
