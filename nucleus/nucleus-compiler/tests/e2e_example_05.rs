//! End-to-end test for example 05-stencil, pthreads-sync backend
//! (TASK-0031).
//!
//! Verifies the full pipeline for the naive schedule:
//!   nucleus build  ->  cargo build  ->  binary run  ->  diff vs reference.bin
//! with bit-identical output (PRD §10.1).
//!
//! The blocked schedule is also covered (active, not `#[ignore]`'d):
//! TASK-0142 landed trailing-remainder tile support, so `block=4` on
//! `y`'s effective range `1..H-1` (= `1..15`, length 14 = 3 full
//! tiles of 4 + a partial tile of 2) is rewritten as a static
//! `Sequence[full-tile nest, trailing partial tile]` rather than
//! rejected with `BlockTransformError::NotDivisible`. NOTE (honest
//! scope): 05-stencil is a single-`host` schedule, so the backend
//! emits from `LinkedIR::algo` source, not the block-transformed
//! ACFG. This cell therefore guards the *compile-doesn't-reject +
//! passes-don't-panic + bit-identical result* property; the numeric
//! correctness of the tiling decomposition is asserted by the
//! `block_transform` unit / integration tests, not by this e2e diff.
//! See TASK-0142 notes and the index-reconstruction follow-up.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Locate the workspace root (the directory containing the top-level
/// `nucleus/Cargo.toml`). Mirrors the helper in `e2e_example_01.rs` /
/// `e2e_example_02.rs` / `e2e_example_03.rs`.
fn repo_root() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(Path::parent)
        .expect("CARGO_MANIFEST_DIR has at least two ancestors")
        .to_path_buf()
}

fn example_dir() -> PathBuf {
    repo_root().join("nuc-nucleus/examples/05-stencil")
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
/// diff) for a given schedule of example 05. Factored so the `naive`
/// and the (currently ignored) `blocked` tests share machinery.
fn run_example_05(sched_rel: &str, scratch_name: &str) {
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
    run_example_05(
        "schedules/naive.sched.nuc",
        "example_05_naive_pthreads_sync",
    );
}

/// Blocked schedule e2e (active since TASK-0142). `block=4` on `y`'s
/// length-14 range is now rewritten to a static full-tile nest plus
/// a trailing partial tile, so the pipeline compiles, builds, runs,
/// and produces bit-identical output vs `reference.bin` (the result
/// is schedule-independent for this single-`host` example). Tiling
/// *structure* correctness is pinned by the `block_transform` tests;
/// this cell pins the end-to-end no-reject / no-panic / bit-identical
/// property.
#[test]
fn blocked_pthreads_sync_bit_identical() {
    run_example_05(
        "schedules/blocked.sched.nuc",
        "example_05_blocked_pthreads_sync",
    );
}
