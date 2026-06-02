//! End-to-end test for example 02-split-add, split schedule,
//! pthreads-sync backend (TASK-0021 target, TASK-0122 enabled).
//!
//! Verifies the full pipeline:
//!   nucleus build  ->  cargo build  ->  binary run  ->  diff vs reference.bin
//! with bit-identical output (PRD §10.1). This is the load-bearing
//! acceptance assertion for multi-worker pthreads-sync codegen.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Locate the workspace root (the directory containing the top-level
/// `nucleus/Cargo.toml`). Mirrors the helper in `e2e_example_01.rs`.
fn repo_root() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(Path::parent)
        .expect("CARGO_MANIFEST_DIR has at least two ancestors")
        .to_path_buf()
}

fn example_dir() -> PathBuf {
    repo_root().join("nuc-nucleus/examples/02-split-add")
}

fn scratch_dir(name: &str) -> PathBuf {
    // TASK-0426.01: per-call-unique leaf (`{name}-{pid}-{counter}`),
    // created once and never removed, so the dev/release `cargo test`
    // processes (which share the profile-independent `target/{name}`
    // path) cannot race remove_dir_all against fs::write. Inlined here
    // because nucleus-compiler does not depend on `test-common`.
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let target = repo_root().join("nucleus/target/e2e-scratch");
    let _ = fs::create_dir_all(&target);
    let nonce = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = target.join(format!("{name}-{}-{}", std::process::id(), nonce));
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

#[test]
fn split_pthreads_sync_bit_identical() {
    let ex = example_dir();
    let algo = ex.join("prog.algo.nuc");
    let sched = ex.join("schedules/split.sched.nuc");
    let kernels = ex.join("kernels.rs");
    let input_bin = ex.join("input.bin");
    let reference_bin = ex.join("reference.bin");

    assert!(algo.exists(), "missing algo at {}", algo.display());
    assert!(sched.exists(), "missing sched at {}", sched.display());
    assert!(kernels.exists(), "missing kernels at {}", kernels.display());
    assert!(input_bin.exists(), "missing input.bin");
    assert!(reference_bin.exists(), "missing reference.bin");

    let out = scratch_dir("example_02_split_pthreads_sync");

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
