//! End-to-end test for example 04-prefix-sum, pthreads-sync backend
//! (TASK-0039).
//!
//! Verifies the full pipeline for the naive schedule:
//!   nucleus build  ->  cargo build  ->  binary run  ->  diff vs reference.bin
//! with bit-identical output (PRD §10.1). The reference is an
//! independent straight-line running-sum oracle (a DIFFERENT
//! algorithm from the program's three-pass block decomposition), so
//! a byte match is a real cross-witness, not a tautology.
//!
//! The `blocked` schedule is shipped but its e2e test is `#[ignore]`'d
//! (NOT silently passing): `loop b : block=2` is evenly divisible,
//! but `b` is the loop variable of all three accumulator passes, and
//! the backend's `divisible_inner_block_vars` count==1 guard (meant
//! for the non-divisible two-nest case, TASK-0173) excludes a
//! reused-name var, so absolute-index rebinding is skipped and the
//! per-block accumulators double-count (output is 2x the reference on
//! both backends). Tracked as TASK-0180. The schedule still parses,
//! lowers, links, and builds — the gate is purely on the (wrong) run
//! result, which is exactly why this stays ignored rather than faked.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Locate the workspace root (the directory containing the top-level
/// `nucleus/Cargo.toml`). Mirrors the helper in the other
/// `e2e_example_*.rs` tests.
fn repo_root() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(Path::parent)
        .expect("CARGO_MANIFEST_DIR has at least two ancestors")
        .to_path_buf()
}

fn example_dir() -> PathBuf {
    repo_root().join("nuc-nucleus/examples/04-prefix-sum")
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
/// diff) for a given schedule of example 04. Factored so the `naive`
/// and the (ignored) `blocked` tests share machinery.
fn run_example_04(sched_rel: &str, scratch_name: &str) {
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
    run_example_04(
        "schedules/naive.sched.nuc",
        "example_04_naive_pthreads_sync",
    );
}

/// Blocked schedule e2e: blocked on TASK-0180. `loop b : block=2`
/// (evenly divisible, NB=4) over a loop variable reused across all
/// three accumulator passes hits the `divisible_inner_block_vars`
/// count==1 guard, so absolute-index rebinding is skipped and the
/// accumulators double-count. The schedule parses/lowers/links/builds;
/// only the run result is wrong, so this stays `#[ignore]`'d rather
/// than asserting an incorrect value. Lift when TASK-0180 lands.
#[test]
#[ignore = "TODO TASK-0180: block= on a loop var reused across passes skips absolute-index rebinding ⇒ accumulator double-count"]
fn blocked_pthreads_sync_bit_identical() {
    run_example_04(
        "schedules/blocked.sched.nuc",
        "example_04_blocked_pthreads_sync",
    );
}
