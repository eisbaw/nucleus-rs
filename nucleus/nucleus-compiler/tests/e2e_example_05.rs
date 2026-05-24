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

/// Reuse schedule e2e (active since TASK-0265 cycle 87): the single-
/// host `reuse.sched.nuc` carries `loop x : reuse;`. Stage 1
/// (TASK-0261) populates `reuse_widths[x_iv][img_in][1] = ReuseSlot
/// {min=-1, length=3}`; Stage 2 Tier 1 (TASK-0265) wires the backend-
/// walker consumer that emits a `reuse_widths_pending` marker comment
/// at the inner-loop body entry. Real circular-buffer codegen is
/// forward-carried to TASK-0265.01..03; the marker is the AC#4
/// "emitted code contains a marker" half. Output stays bit-identical
/// to reference.bin (the marker is a comment).
#[test]
fn reuse_pthreads_sync_bit_identical() {
    run_example_05(
        "schedules/reuse.sched.nuc",
        "example_05_reuse_pthreads_sync",
    );
}

/// AC#4 marker-detection: the emitted main.rs for the `reuse` schedule
/// MUST contain the `reuse_widths_pending` marker substring at LEAST
/// once. Detection is grep-based, identical to how the broader
/// codegen test crates assert specific emit shapes (e.g.
/// `eventlist_alone_reconstructs_stencil_kernel_call`). If a future
/// refactor drops `render_reuse_marker_comment`, this test fires
/// LOUD instead of silently re-introducing the Stage-1 ⇒ Stage-2
/// reuse-widths blind spot.
///
/// Symmetric ABSENCE check: the `naive` schedule (no reuse directive)
/// MUST NOT contain the marker — guards against an over-eager emit
/// that fires the marker when no reuse slot exists.
#[test]
fn reuse_marker_present_on_reuse_schedule_absent_on_naive() {
    // Reuse schedule: marker must appear at least once on the inner
    // x-loop. (We rely on `run_example_05` having just run via
    // `reuse_pthreads_sync_bit_identical`; but that ordering is not
    // guaranteed by cargo test, so re-run the build here to get a
    // fresh main.rs we can grep.)
    let reuse_dir = scratch_dir("example_05_reuse_marker_check");
    let ex = example_dir();
    let nuc_ws = repo_root().join("nucleus");

    let reuse_build = Command::new("cargo")
        .arg("run")
        .arg("--quiet")
        .arg("--bin")
        .arg("nucleus")
        .arg("--")
        .arg("build")
        .arg("--algo")
        .arg(ex.join("prog.algo.nuc"))
        .arg("--sched")
        .arg(ex.join("schedules/reuse.sched.nuc"))
        .arg("--kernels")
        .arg(ex.join("kernels.rs"))
        .arg("--backend")
        .arg("pthreads-sync")
        .arg("--out")
        .arg(&reuse_dir)
        .current_dir(&nuc_ws)
        .output()
        .expect("nucleus build on reuse schedule");
    assert!(
        reuse_build.status.success(),
        "nucleus build (reuse) failed:\n{}",
        String::from_utf8_lossy(&reuse_build.stderr)
    );
    let reuse_main = fs::read_to_string(reuse_dir.join("src/main.rs"))
        .expect("read main.rs (reuse build)");
    let reuse_count = reuse_main.matches("reuse_widths_pending").count();
    assert!(
        reuse_count >= 1,
        "TASK-0265 AC#4: reuse schedule's emitted main.rs MUST contain at \
         least one `reuse_widths_pending` marker; got {reuse_count}.\n\
         If this firing dropped, render_reuse_marker_comment regressed."
    );
    // Pin the exact slot the inference recovered:
    //   x_iv carries reuse; img_in[y][x±1] reads ⇒ length=3, min_offset=-1.
    assert!(
        reuse_main.contains("iv=x"),
        "marker must name iv=x (the only reuse-tagged loop)"
    );
    assert!(
        reuse_main.contains("data=img_in"),
        "marker must name data=img_in (the only DataRef whose x-axis offsets are non-degenerate)"
    );
    assert!(
        reuse_main.contains("axis=1"),
        "marker must name axis=1 (x is the inner axis of img_in[y][x])"
    );
    assert!(
        reuse_main.contains("length=3"),
        "marker must name length=3 (offsets {{-1, 0, +1}} ⇒ length 3)"
    );
    assert!(
        reuse_main.contains("min_offset=-1"),
        "marker must name min_offset=-1 (smallest offset is -1)"
    );

    // Naive schedule: marker MUST NOT appear (no reuse directive).
    let naive_dir = scratch_dir("example_05_naive_marker_absent_check");
    let naive_build = Command::new("cargo")
        .arg("run")
        .arg("--quiet")
        .arg("--bin")
        .arg("nucleus")
        .arg("--")
        .arg("build")
        .arg("--algo")
        .arg(ex.join("prog.algo.nuc"))
        .arg("--sched")
        .arg(ex.join("schedules/naive.sched.nuc"))
        .arg("--kernels")
        .arg(ex.join("kernels.rs"))
        .arg("--backend")
        .arg("pthreads-sync")
        .arg("--out")
        .arg(&naive_dir)
        .current_dir(&nuc_ws)
        .output()
        .expect("nucleus build on naive schedule");
    assert!(
        naive_build.status.success(),
        "nucleus build (naive) failed:\n{}",
        String::from_utf8_lossy(&naive_build.stderr)
    );
    let naive_main = fs::read_to_string(naive_dir.join("src/main.rs"))
        .expect("read main.rs (naive build)");
    let naive_count = naive_main.matches("reuse_widths_pending").count();
    assert_eq!(
        naive_count, 0,
        "naive schedule has no `reuse` directive ⇒ marker MUST NOT appear; \
         got {naive_count} occurrence(s). Symmetric ABSENCE check guards \
         against an over-eager render_reuse_marker_comment that fires when \
         no slot exists."
    );
}
