//! End-to-end test for example 01-elementwise-add, naive schedule,
//! pthreads-sync backend (TASK-0020 acceptance test).
//!
//! Pipeline tested:
//!   nucleus build  ->  cargo build  ->  binary run  ->  diff vs reference.bin
//!
//! Bit-identical output is non-negotiable (PRD §10.1).
//!
//! Why this test lives in `compiler/tests/` rather than the
//! pthreads-sync backend crate's tests: at M1 the backend crate
//! depends on `compiler`, but the e2e flow drives the full pipeline
//! including the binary (`nucleus`) in the `driver` crate. Tests in
//! either backend or driver would create awkward dev-dependency
//! cycles. The compiler crate is the common ancestor; the test
//! invokes the `nucleus` binary via `cargo run`.
//!
//! NB: this is a CI-friendly test in the sense that it only assumes
//! the workspace is on disk and `cargo` is on PATH (true inside
//! `nix develop`). It compiles the generated project with cargo,
//! which means the test is slow-ish (~10s on a developer laptop);
//! still cheap enough to run on every `just test`.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Locate the workspace root (the directory containing the top-level
/// `nucleus/Cargo.toml`). We resolve by walking up from this test
/// binary's location at compile time using `CARGO_MANIFEST_DIR`.
fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR points to `nucleus/compiler`.
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // ../.. is the repo root.
    here.parent()
        .and_then(Path::parent)
        .expect("CARGO_MANIFEST_DIR has at least two ancestors")
        .to_path_buf()
}

fn example_dir() -> PathBuf {
    repo_root().join("nuc-nucleus/examples/01-elementwise-add")
}

/// Path to a unique scratch directory per test invocation. We use the
/// crate's `target/` so that `cargo clean` sweeps generated artefacts
/// without manual rm-rf.
fn scratch_dir(name: &str) -> PathBuf {
    let target = repo_root().join("nucleus/target/e2e-scratch");
    let _ = fs::create_dir_all(&target);
    let dir = target.join(name);
    // Idempotent: a previous run may have left state behind. Remove
    // and recreate so each invocation starts clean.
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

#[test]
fn example_01_naive_pthreads_sync_bit_identical() {
    let ex = example_dir();
    let algo = ex.join("prog.algo.nuc");
    let sched = ex.join("schedules/naive.sched.nuc");
    let kernels = ex.join("kernels.rs");
    let input_bin = ex.join("input.bin");
    let reference_bin = ex.join("reference.bin");

    assert!(algo.exists(), "missing algo at {}", algo.display());
    assert!(sched.exists(), "missing sched at {}", sched.display());
    assert!(kernels.exists(), "missing kernels at {}", kernels.display());
    assert!(input_bin.exists(), "missing input.bin");
    assert!(reference_bin.exists(), "missing reference.bin");

    let out = scratch_dir("example_01_naive_pthreads_sync");

    // ---- Step 1: invoke `nucleus build`. We use `cargo run` so the
    // test does not depend on the binary being pre-built. The
    // workspace is at nucleus/ so we invoke from there. ----
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

    // ---- Step 2: cargo build inside the generated project. ----
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

    // ---- Step 3: run the generated binary against input.bin. ----
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

    // ---- Step 4: diff vs reference.bin. ----
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
