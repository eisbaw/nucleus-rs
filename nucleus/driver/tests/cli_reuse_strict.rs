//! Integration test for TASK-0274: pin the driver-level wrapping of
//! `ReuseInferenceError` (TASK-0271 cycle 88 promoted
//! `apply_reuse_inference_advisory` → `apply_reuse_inference` at
//! `driver/src/main.rs:413` and wrapped the typed error via
//! `.map_err(|e| format!("reuse-inference error: {e}"))?`).
//!
//! The pass-level pin
//! `nucleus-compiler/tests/sidecar_reuse.rs::task0271_strict_rejects_non_affine_reuse_body`
//! verifies the strict variant returns the typed Err. THIS test pins
//! the CLI's outer contract — the wrapped error string AND the
//! variant Display text both surface to the user. A future refactor
//! that drops the "reuse-inference error:" prefix or swaps `format!`
//! for `to_string()` would silently change the user-visible
//! diagnostic; this test bites in that case.
//!
//! Subprocess pattern mirrors `nucleus/driver/tests/emit_pn.rs`.

use std::path::PathBuf;
use std::process::Command;

/// Walk up from `CARGO_MANIFEST_DIR` until we find the repo root.
/// Same idiom as `emit_pn.rs`.
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

fn nucleus_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_nucleus"))
}

fn fixture_dir() -> PathBuf {
    repo_root()
        .join("nucleus")
        .join("driver")
        .join("tests")
        .join("fixtures")
        .join("task_0274_strided_reuse")
}

fn fresh_outdir(tag: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("nucleus-cli-reuse-strict-{}-{}", tag, std::process::id()));
    if p.exists() {
        let _ = std::fs::remove_dir_all(&p);
    }
    std::fs::create_dir_all(&p).expect("create tempdir");
    p
}

#[test]
fn nucleus_build_fails_loud_on_strided_reuse_with_wrapped_diagnostic() {
    let fx = fixture_dir();
    let out = fresh_outdir("strided");

    let result = Command::new(nucleus_bin())
        .arg("build")
        .arg("--algo")
        .arg(fx.join("prog.algo.nuc"))
        .arg("--sched")
        .arg(fx.join("sched.nuc"))
        .arg("--kernels")
        .arg(fx.join("kernels.rs"))
        .arg("--backend")
        .arg("pthreads-sync")
        .arg("--out")
        .arg(&out)
        .output()
        .expect("spawn nucleus");

    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);

    // 1. Exit code MUST be non-zero.
    assert!(
        !result.status.success(),
        "expected nucleus build to FAIL on strided reuse fixture, \
         but it succeeded.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    // 2. The driver's wrapping prefix MUST be present somewhere in
    //    the output (stderr OR stdout — the wrapping target depends
    //    on how main.rs surfaces the propagated `?`).
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("reuse-inference error:"),
        "expected driver-wrapped prefix `reuse-inference error:` in CLI output \
         (TASK-0271 cycle 88 contract).\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    // 3. The variant-specific Display text MUST also be present, so
    //    a future refactor that keeps the prefix but loses the inner
    //    typed-error message also fails LOUD. The strided fixture
    //    triggers `ReuseInferenceError::StridedAccessNotSupported`
    //    whose Display contains "strided" and "coefficient 2".
    assert!(
        combined.contains("strided"),
        "expected variant Display substring `strided` in CLI output \
         (ReuseInferenceError::StridedAccessNotSupported, see \
         reuse_inference.rs:322).\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        combined.contains("coefficient 2"),
        "expected recovered-coefficient `coefficient 2` in CLI output \
         (the strided index is `src[v * 2]`).\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}
