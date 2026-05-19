//! Multi-worker pthreads-sync codegen tests (TASK-0122 AC #1).
//!
//! Drives a synthetic two-worker pingpong scenario end-to-end:
//! parse + lower + link + ACFG + sync + transfer injection -> emit
//! to a tempdir -> `cargo build` the generated project -> run the
//! binary -> diff its output against an expected value.
//!
//! The driving algorithm here is *not* example 02 (that lives in
//! `nucleus/compiler/tests/e2e_example_02.rs`); we use a smaller,
//! purpose-built two-worker case that fits in a unit-test file so
//! the multi-worker rejection / codegen path can be exercised in
//! isolation from the example matrix.
//!
//! Why a real example would also work: we could just point this
//! test at example 02, but that would duplicate the e2e_example_02
//! coverage. Here we stress a smaller scenario whose kernels are
//! self-contained — `produce` returns a constant `Vec<i32>`,
//! `consume` writes a checksum into `NUC_OUTPUT_PATH`.
//!
//! Limitation: building a fresh Cargo project per test is slow
//! (~1.5 s). We don't gate this behind `#[ignore]` because the
//! multi-worker codegen is M1's load-bearing capability — the test
//! must run on every commit.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use compiler::{
    acfg_to_events,
    algo::{lower_algo, parse_algo},
    build_acfg, build_sidecar, inject_syncs, inject_transfers, link,
    sched::{lower_sched, parse_sched},
};
use pthreads_sync::{emit, NameTables};

fn repo_root() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .expect("three ancestors above pthreads-sync crate")
        .to_path_buf()
}

fn scratch_dir(name: &str) -> PathBuf {
    let target = repo_root().join("nucleus/target/pthreads-sync-multi-worker-scratch");
    let _ = fs::create_dir_all(&target);
    let dir = target.join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Tiny synthetic algorithm: host produces, w0 consumes, host
/// summarises. Three Push/Wait pairs — exactly the AC #1 shape.
///
/// Dataflow:
///   x <-- produce_x();       // on host
///   y <-- produce_y();       // on host
///   z <-- combine(x, y);     // on w0  -- consumes x, y; produces z
///   sink(z);                 // on host -- consumes z (effect)
///
/// All three data symbols (x, y, z) cross workers; AC #1 names
/// "three Push/Wait pairs".
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

/// Hand-written kernels for the synthetic test. `produce_x` returns
/// [0, 1, ..., 15]; `produce_y` returns [100, 100, ..., 100];
/// `combine` is element-wise wrapping add; `sink` writes the
/// expected sum (in this case 1320 = 0+1+...+15 + 16*100) to
/// `NUC_OUTPUT_PATH` as a single little-endian i32.
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

/// Expected sum: 0+1+...+15 + 16*100 = 120 + 1600 = 1720.
const EXPECTED_SUM: i32 = 120 + 1600;

#[test]
fn two_worker_pingpong_compiles_and_runs() {
    let scratch = scratch_dir("two_worker_pingpong");

    // Write the algo / sched / kernels.rs to the scratch dir so the
    // backend has real files to point at.
    let algo_path = scratch.join("prog.algo.nuc");
    let sched_path = scratch.join("prog.sched.nuc");
    let kernels_path = scratch.join("kernels.rs");
    fs::write(&algo_path, ALGO_SRC).unwrap();
    fs::write(&sched_path, SCHED_SRC).unwrap();
    fs::write(&kernels_path, KERNELS_SRC).unwrap();

    // Drive the pipeline.
    let algo_ast = parse_algo(ALGO_SRC).expect("algo parse");
    let sched_ast = parse_sched(SCHED_SRC).expect("sched parse");
    let algo_ir = lower_algo(&algo_ast).expect("algo lower");
    let sched_ir = lower_sched(&sched_ast).expect("sched lower");
    let linked = link(algo_ir, sched_ir).expect("link");
    let acfg = build_acfg(&linked).expect("build_acfg");
    let acfg = inject_syncs(acfg);
    let acfg = inject_transfers(&linked, acfg);

    let out_dir = scratch.join("gen");
    // TASK-0124 contract path: project to per-worker EventList +
    // build sidecar + reverse name tables, exactly as the driver.
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
    let result =
        emit(&per_worker, &names, &sidecar, &kernels_path, &out_dir).expect("emit succeeded");

    // Verify the generated main.rs structurally mentions the
    // expected primitives.
    let main_rs = fs::read_to_string(&result.main_rs).unwrap();
    for needle in &[
        "thread::spawn",
        "Slot<Vec<i32>>",
        "Barrier::new",
        ".wait()",
        ".push(",
        "kernels::produce_x",
        "kernels::combine",
        "kernels::sink",
    ] {
        assert!(
            main_rs.contains(needle),
            "main.rs missing expected snippet `{needle}`:\n{main_rs}",
        );
    }

    // Build the generated project.
    let build = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--quiet")
        .current_dir(&out_dir)
        .output()
        .expect("cargo build on generated project");
    assert!(
        build.status.success(),
        "generated project failed to build:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr),
    );

    // Run the binary; sink will write the i32 sum to out_path.
    let out_bin = out_dir.join("output.bin");
    let exe = out_dir.join("target/release/nuc-generated");
    assert!(exe.exists(), "expected binary at {}", exe.display());
    let run = Command::new(&exe)
        .env("NUC_OUTPUT_PATH", &out_bin)
        .output()
        .expect("run generated binary");
    assert!(
        run.status.success(),
        "generated binary returned non-zero:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
    );

    let bytes = fs::read(&out_bin).expect("read output.bin");
    assert_eq!(
        bytes.len(),
        4,
        "expected 4-byte i32 sum, got {}",
        bytes.len()
    );
    let got = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    assert_eq!(got, EXPECTED_SUM, "pingpong sum mismatch");
}
