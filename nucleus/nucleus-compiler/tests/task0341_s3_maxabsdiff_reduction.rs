//! TASK-0341.02.01.04 (grammar-epic S3) — end-to-end bit-identity
//! proof for a SCALAR-PRODUCING max-abs-diff (L-infinity) reduction
//! over a generation pair of i32 arrays.
//!
//! ## Why this test exists — premise verification, not new machinery
//!
//! The S3 task body asserted "NO existing machinery to reuse" for a
//! scalar-producing reduction. That premise is FALSE, and this test
//! is the empirical disproof. The fixture
//! (`tests/fixtures/task0341_s3_maxabsdiff/`) is expressed entirely in
//! shapes that already ship bit-identical in the e2e matrix:
//!   * 03-reduction's two-phase reduction (per-partition accumulate
//!     into `partials[w]`, then a tree-combine of partials to a
//!     scalar) — this fixture is that shape with the fold op changed
//!     from `+` to `max`.
//!   * 07-matmul's 3-arg pure accumulator `c[i][j] <-- madd(c[i][j],
//!     a[i][k], b[k][j])` — `max_abs_acc(partials[w], new[w][i],
//!     old[w][i])` is structurally identical.
//!
//! No grammar, IR, or codegen change was required — only kernel bodies.
//!
//! ## The max-identity crux
//!
//! General min/max reductions need their identity (INT_MIN/INT_MAX)
//! materialised, which v2's zero-init pre-init pass cannot supply (see
//! 03-reduction's README). BUT max-abs-diff is special: every
//! per-element `abs(new - old) >= 0`, so 0 IS the correct max-identity
//! (`max(0, nonneg) == nonneg`). The codegen's existing zero-init of
//! `partials` (the additive identity for sum) is therefore ALSO the
//! correct identity for this max reduction — no `init=` clause needed.
//!
//! ## What this proves (AC#1)
//!
//! The full pipeline `nucleus build -> cargo build -> run -> diff`
//! produces bit-identical output vs `reference.bin` (the hand-computed
//! scalar max-abs-diff = 186, independently derived in
//! `cruft/spike_s3_ref.py` and checked there against a direct
//! order-free `max(abs(new[i]-old[i]))`). Bit-identity is PRD §10.1.
//!
//! ## Scope
//!
//! This is the REDUCTION in isolation under a single-worker schedule.
//! The generation pair is two top-level input arrays (NOT an
//! until-loop — that is S1/S4). The cross-schedule / cross-backend
//! differential for the collective break is S7 (TASK-0341.02.01.08).
//! Order-independence of the scalar (norm = max) is pinned separately
//! by `task0341_s3_order_independence.rs` (AC#2). Lives in the
//! nucleus-compiler test crate so it runs under `just test` and does
//! NOT perturb the `just e2e` matrix baseline (420/363/0/57/0).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Locate the workspace root (the directory containing the top-level
/// `nucleus/Cargo.toml`). Mirrors `e2e_example_03.rs`.
fn repo_root() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(Path::parent)
        .expect("CARGO_MANIFEST_DIR has at least two ancestors")
        .to_path_buf()
}

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/task0341_s3_maxabsdiff")
}

/// Per-call-unique scratch leaf, created once and never removed, so the
/// dev/release `cargo test` processes (sharing the profile-independent
/// `target/{name}` path) cannot race remove_dir_all against fs::write.
/// Identical idiom to `e2e_example_03.rs::scratch_dir` (TASK-0426.01).
fn scratch_dir(name: &str) -> PathBuf {
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
fn naive_pthreads_sync_bit_identical() {
    let ex = fixture_dir();
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

    let out = scratch_dir("task0341_s3_maxabsdiff_naive");

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
        "nucleus build failed — S3 PREMISE-VERIFICATION FAILURE: if this is a \
         lower/link/emit error, the scalar-producing max-abs-diff reduction is \
         NOT expressible on existing machinery and the gap is a real finding.\n\
         stdout:\n{}\nstderr:\n{}",
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
    assert!(exe.exists(), "expected generated binary at {}", exe.display());

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
        actual, expected,
        "S3 max-abs-diff reduction output is not bit-identical to reference.bin \
         (expected scalar 186 = 0x000000ba LE). actual={actual:?} expected={expected:?}"
    );
}
