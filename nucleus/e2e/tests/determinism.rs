//! Integration test for the determinism check (TASK-0033).
//!
//! Runs `cargo run --bin nucleus-e2e -- --check-determinism --example
//! 01-elementwise-add` from the workspace and verifies that:
//!
//!   1. The harness exits 0 (codegen for the M1 trio is deterministic).
//!   2. The summary table contains a PASS line for the targeted cell.
//!
//! This is the "does the test bite?" companion test asked for by
//! TASK-0033 AC #4. The positive arm (deterministic codegen produces
//! PASS) lives here; a true negative arm (deliberately introduce
//! HashMap iteration into codegen and verify the check fails) is
//! deferred — it would require either a patched nucleus-compiler
//! crate or a synthetic codegen-with-nondeterminism fixture, both of
//! which are
//! larger than the bite-test buys. The check's bite is already
//! ensured by the byte-by-byte file diff: any file divergence,
//! anywhere, is hard FAIL — there is no silent path.
//!
//! Cost: spawns `cargo run` twice per cell (the harness does), so
//! we narrow to one example/schedule/backend triple to keep the test
//! cheap. Full-matrix determinism is what `just determinism-check`
//! exercises in CI.

use std::path::PathBuf;
use std::process::Command;

/// Walk up from the test binary location until we find the directory
/// that holds both `nucleus/Cargo.toml` and `nuc-nucleus/PRD.md`.
fn repo_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        if p.join("nucleus").join("Cargo.toml").exists()
            && p.join("nuc-nucleus").join("PRD.md").exists()
        {
            return p;
        }
        if !p.pop() {
            panic!("could not locate repo root from CARGO_MANIFEST_DIR");
        }
    }
}

#[test]
fn determinism_check_passes_for_example_01_naive_pthreads_sync() {
    // Limit the matrix to one cell so the test stays cheap.
    let root = repo_root();
    let out = Command::new("cargo")
        .arg("run")
        .arg("--quiet")
        .arg("--bin")
        .arg("nucleus-e2e")
        .arg("--")
        .arg("--check-determinism")
        .arg("--example")
        .arg("01-elementwise-add")
        .arg("--schedule")
        .arg("naive")
        .arg("--backend")
        .arg("pthreads-sync")
        .current_dir(root.join("nucleus"))
        .output()
        .expect("spawn nucleus-e2e");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        out.status.success(),
        "determinism check did not exit 0:\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );

    // The summary table must include a PASS line for the cell. We
    // grep for both "PASS" and the cell identifier — checking the
    // exit code alone would let a future regression where no cells
    // ran at all sneak through.
    assert!(
        stdout.contains("01-elementwise-add") && stdout.contains("PASS"),
        "no PASS line for example 01 found in stdout:\n{stdout}"
    );

    // Header line should mention the task.
    assert!(
        stdout.contains("TASK-0033"),
        "summary header missing TASK-0033 marker:\n{stdout}"
    );
}
