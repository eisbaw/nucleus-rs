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

use compiler::{
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
fn pipeline(scratch: &Path) -> (PathBuf, NameTables, compiler::sidecar::NameSidecar,
    std::collections::BTreeMap<compiler::event::WorkerId, Vec<compiler::event::Event>>) {
    let kernels_path = scratch.join("kernels.rs");
    fs::write(&kernels_path, KERNELS_SRC).unwrap();

    let algo_ast = parse_algo(ALGO_SRC).expect("algo parse");
    let sched_ast = parse_sched(SCHED_SRC).expect("sched parse");
    let algo_ir = lower_algo(&algo_ast).expect("algo lower");
    let sched_ir = lower_sched(&sched_ast).expect("sched lower");
    let linked = link(algo_ir, sched_ir).expect("link");
    let acfg = build_acfg(&linked).expect("build_acfg");
    let acfg = inject_syncs(acfg);
    let acfg = inject_transfers(&linked, acfg);

    let per_worker = acfg_to_events(&acfg);
    let sidecar = build_sidecar(&linked, &acfg).expect("build_sidecar");
    let names = NameTables {
        data: acfg.name_data.iter().map(|(n, i)| (*i, n.clone())).collect(),
        kernel: acfg
            .name_kernels
            .iter()
            .map(|(n, i)| (*i, n.clone()))
            .collect(),
        worker: acfg
            .name_workers
            .iter()
            .map(|(n, i)| (*i, n.clone()))
            .collect(),
        iter_var: acfg
            .name_iter_vars
            .iter()
            .map(|(n, i)| (*i, n.clone()))
            .collect(),
        inner_block_iter_vars: acfg.inner_block_iter_vars.clone(),
    };
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
    for needle in &["wire::write_msg", "wire::read_msg_expect", "wire::barrier_cross"] {
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
