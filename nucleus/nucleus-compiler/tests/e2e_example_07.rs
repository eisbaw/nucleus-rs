//! End-to-end test for example 07-matmul, pthreads-sync backend
//! (TASK-0032).
//!
//! Verifies the full pipeline for the naive schedule:
//!   nucleus build  ->  cargo build  ->  binary run  ->  diff vs reference.bin
//! with bit-identical output (PRD §10.1).
//!
//! With TASK-0143 (per-tile Push/Wait hoisting) landed and the
//! block-transform pass (TASK-0030) producing the correct
//! (i__tile, i, j__tile, j, k) nest at N=16 with block=8 on both
//! outer axes, the blocked cell now produces a bit-identical
//! output and is no longer `#[ignore]`'d. The schedule has a single
//! `host` worker, so the hoisting is a no-op at this cell but the
//! generated code still has to come out byte-for-byte the same as
//! the naive schedule's reference — which it does.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Locate the workspace root (the directory containing the top-level
/// `nucleus/Cargo.toml`). Mirrors the helper in `e2e_example_01.rs` /
/// `e2e_example_02.rs` / `e2e_example_03.rs` / `e2e_example_05.rs`.
fn repo_root() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(Path::parent)
        .expect("CARGO_MANIFEST_DIR has at least two ancestors")
        .to_path_buf()
}

fn example_dir() -> PathBuf {
    repo_root().join("nuc-nucleus/examples/07-matmul")
}

fn scratch_dir(name: &str) -> PathBuf {
    let target = repo_root().join("nucleus/target/e2e-scratch");
    let _ = fs::create_dir_all(&target);
    let dir = target.join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Drives the full pipeline (nucleus build -> cargo build -> run ->
/// diff) for a given schedule of example 07. Factored so the `naive`
/// and the (currently ignored) `blocked` tests share machinery.
fn run_example_07(sched_rel: &str, scratch_name: &str) {
    let ex = example_dir();
    let algo = ex.join("prog.algo.nuc");
    let sched = ex.join(sched_rel);
    let kernels = ex.join("kernels.rs");
    let input_bin = ex.join("input.bin");
    let reference_bin = ex.join("reference.bin");

    assert!(algo.exists(), "missing algo at {}", algo.display());
    assert!(sched.exists(), "missing sched at {}", sched.display());
    assert!(kernels.exists(), "missing kernels at {}", kernels.display());
    assert!(input_bin.exists(), "missing input.bin");
    assert!(reference_bin.exists(), "missing reference.bin");

    let out = scratch_dir(scratch_name);

    let nuc_ws = repo_root().join("nucleus");
    let build_out = Command::new("cargo")
        .arg("run")
        .arg("--quiet")
        .arg("--bin")
        .arg("nucleus")
        .arg("--")
        .arg("build")
        .arg("--algo")
        .arg(&algo)
        .arg("--sched")
        .arg(&sched)
        .arg("--kernels")
        .arg(&kernels)
        .arg("--backend")
        .arg("pthreads-sync")
        .arg("--out")
        .arg(&out)
        .current_dir(&nuc_ws)
        .output()
        .expect("failed to invoke `cargo run --bin nucleus`");

    assert!(
        build_out.status.success(),
        "nucleus build failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&build_out.stdout),
        String::from_utf8_lossy(&build_out.stderr)
    );

    let gen_build = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--quiet")
        .current_dir(&out)
        .output()
        .expect("failed to run cargo build on generated project");

    assert!(
        gen_build.status.success(),
        "cargo build on generated project failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&gen_build.stdout),
        String::from_utf8_lossy(&gen_build.stderr)
    );

    let output_bin = out.join("output.bin");
    let exe = out.join("target/release/nuc-generated");
    assert!(
        exe.exists(),
        "expected generated binary at {}",
        exe.display()
    );
    let run_out = Command::new(&exe)
        .env("NUC_INPUT_PATH", &input_bin)
        .env("NUC_OUTPUT_PATH", &output_bin)
        .output()
        .expect("failed to run generated binary");
    assert!(
        run_out.status.success(),
        "generated binary returned non-zero:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&run_out.stdout),
        String::from_utf8_lossy(&run_out.stderr)
    );

    let expected = fs::read(&reference_bin).expect("read reference.bin");
    let actual = fs::read(&output_bin).expect("read generated output.bin");
    assert_eq!(
        actual.len(),
        expected.len(),
        "output length {} != reference length {}",
        actual.len(),
        expected.len()
    );
    assert_eq!(
        actual, expected,
        "generated output is not bit-identical to reference.bin"
    );
}

#[test]
fn naive_pthreads_sync_bit_identical() {
    run_example_07(
        "schedules/naive.sched.nuc",
        "example_07_naive_pthreads_sync",
    );
}

/// Blocked schedule e2e. N=16, block=8 on both i and j: the
/// block-transform pass accepts both rewrites cleanly (TASK-0030
/// divisibility check satisfied) and TASK-0143's per-tile Push/Wait
/// hoisting is now in place. With a single-`host` schedule there
/// are no cross-worker transfers to hoist, so the hoisting pass is
/// a structural identity here; the test still pins that the
/// blocked-schedule pipeline produces byte-identical output to the
/// naive cell's reference.
#[test]
fn blocked_pthreads_sync_bit_identical() {
    run_example_07(
        "schedules/blocked.sched.nuc",
        "example_07_blocked_pthreads_sync",
    );
}
