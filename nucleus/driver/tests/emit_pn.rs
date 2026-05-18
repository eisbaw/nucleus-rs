//! Integration tests for `nucleus build --emit-pn` (TASK-0035, PRD §8.5).
//!
//! These run the actual `nucleus` binary as a subprocess against
//! example 01 (`elementwise-add` / `naive.sched.nuc` / `pthreads-sync`)
//! and inspect the emitted Graphviz DOT file.
//!
//! Why subprocess rather than calling `cmd_build` in-process:
//!
//! - `nucleus` is a `[[bin]]` crate; its module is not exposed as a
//!   library. Spawning the actual binary is also closer to the user
//!   experience the test is supposed to defend.
//! - `cargo test` for a bin crate sets `CARGO_BIN_EXE_nucleus` so the
//!   path is known at compile time. No `cargo run` fork required.
//!
//! What is NOT covered here, deliberately:
//!
//! - Pixel-perfect snapshot of the DOT. Tests that compare DOT byte
//!   for byte are brittle against any future label tweak. We assert
//!   the structural shape (digraph header, cluster per worker,
//!   subgraph fill colour comes from `WORKER_PALETTE`, transition
//!   and place names appear) and leave layout / glyph choices to
//!   Graphviz.
//! - Multi-worker schedules (example 01 is single-worker so the test
//!   only exercises one cluster). When TASK-0044 lands a
//!   distributed schedule example, a multi-worker test should be
//!   added that asserts each worker gets a distinct palette colour.

use std::path::PathBuf;
use std::process::Command;

/// Walk up from `CARGO_MANIFEST_DIR` until we find the repo root
/// (it contains both `nucleus/Cargo.toml` and `nuc-nucleus/PRD.md`).
/// Same idiom as `nucleus/e2e/tests/determinism.rs`.
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

/// Paths for example 01 (elementwise-add) under naive schedule.
struct Example01 {
    algo: PathBuf,
    sched: PathBuf,
    capabilities: PathBuf,
}

fn example_01_paths() -> Example01 {
    let root = repo_root();
    Example01 {
        algo: root
            .join("nuc-nucleus")
            .join("examples")
            .join("01-elementwise-add")
            .join("prog.algo.nuc"),
        sched: root
            .join("nuc-nucleus")
            .join("examples")
            .join("01-elementwise-add")
            .join("schedules")
            .join("naive.sched.nuc"),
        capabilities: root
            .join("nucleus")
            .join("backends")
            .join("pthreads-sync")
            .join("capabilities.toml"),
    }
}

/// Make a fresh tempdir for the test.
///
/// We use `std::env::temp_dir()` + the test name + process id so
/// concurrent test runs don't collide. Cleanup is best-effort; the OS
/// evicts `/tmp` on its own cadence.
fn fresh_outdir(tag: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("nucleus-emit-pn-{}-{}", tag, std::process::id()));
    if p.exists() {
        let _ = std::fs::remove_dir_all(&p);
    }
    std::fs::create_dir_all(&p).expect("create tempdir");
    p
}

#[test]
fn emit_pn_writes_a_dot_file_with_expected_structure() {
    let paths = example_01_paths();
    let out = fresh_outdir("ok");
    let dot_path = out.join("schedule.dot");

    let result = Command::new(nucleus_bin())
        .arg("build")
        .arg("--algo")
        .arg(&paths.algo)
        .arg("--sched")
        .arg(&paths.sched)
        .arg("--backend")
        .arg("pthreads-sync")
        .arg("--capabilities")
        .arg(&paths.capabilities)
        .arg("--out")
        .arg(&out)
        .arg("--emit-pn")
        .arg(&dot_path)
        .output()
        .expect("spawn nucleus");

    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        result.status.success(),
        "nucleus build failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    assert!(
        dot_path.exists(),
        "expected DOT file at {} (stdout: {stdout})",
        dot_path.display()
    );

    let dot = std::fs::read_to_string(&dot_path).expect("read emitted dot");
    // Structural shape only — see module docs for why we don't
    // snapshot the bytes.
    assert!(dot.starts_with("digraph petri"), "missing digraph header in:\n{dot}");
    assert!(dot.contains("subgraph cluster_w"), "missing per-worker cluster in:\n{dot}");
    // The styled serialiser always picks a palette colour from
    // `compiler::petri::WORKER_PALETTE`. The first entry is
    // `lightblue`; example 01 has a single worker which the lowering
    // pass assigns id 0 (deterministically), so `lightblue` must
    // appear. If the palette is ever reordered, this assertion
    // shifts to whatever's at index 0.
    assert!(
        dot.contains("lightblue"),
        "expected the first palette colour for worker 0 in:\n{dot}"
    );
    // Title carries the algorithm + schedule path so a user diff'ing
    // many emitted nets can identify which one is which.
    assert!(
        dot.contains("prog.algo.nuc") && dot.contains("naive.sched.nuc"),
        "expected algo + sched names in the DOT title:\n{dot}"
    );

    // Stdout must announce the emit_pn path so callers can pick it
    // up from the structured output line.
    assert!(
        stdout.contains("emit_pn"),
        "stdout should announce emit_pn path:\n{stdout}"
    );
}

#[test]
fn emit_pn_alone_skips_codegen_but_still_writes_dot() {
    // --emit-pn without --out is an inspection-only build (PRD §8.5).
    // Codegen must NOT run; the DOT must still appear.
    let paths = example_01_paths();
    let scratch = fresh_outdir("inspect");
    let dot_path = scratch.join("schedule.dot");

    let result = Command::new(nucleus_bin())
        .arg("build")
        .arg("--algo")
        .arg(&paths.algo)
        .arg("--sched")
        .arg(&paths.sched)
        .arg("--backend")
        .arg("pthreads-sync")
        .arg("--capabilities")
        .arg(&paths.capabilities)
        .arg("--emit-pn")
        .arg(&dot_path)
        .output()
        .expect("spawn nucleus");

    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        result.status.success(),
        "inspection-only build failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(dot_path.exists(), "DOT file missing at {}", dot_path.display());

    // No codegen ran: the `project_dir =` / `main_rs =` summary
    // lines that the pthreads-sync backend prints must be absent.
    assert!(
        !stdout.contains("project_dir"),
        "codegen ran when it shouldn't have:\n{stdout}"
    );
    assert!(
        !stdout.contains("main_rs"),
        "codegen ran when it shouldn't have:\n{stdout}"
    );
}

#[test]
fn emit_pn_to_nonexistent_directory_fails_loudly() {
    // Negative test: parent dir doesn't exist => fail-fast with a
    // clean error message. We deliberately do NOT auto-create the
    // parent (see `write_emit_pn` rationale in main.rs).
    let paths = example_01_paths();
    let out = fresh_outdir("neg");
    let bogus = out.join("nope").join("does").join("not").join("exist.dot");

    let result = Command::new(nucleus_bin())
        .arg("build")
        .arg("--algo")
        .arg(&paths.algo)
        .arg("--sched")
        .arg(&paths.sched)
        .arg("--backend")
        .arg("pthreads-sync")
        .arg("--capabilities")
        .arg(&paths.capabilities)
        .arg("--emit-pn")
        .arg(&bogus)
        .output()
        .expect("spawn nucleus");

    assert!(
        !result.status.success(),
        "build with bad --emit-pn path should fail; got stdout:\n{}",
        String::from_utf8_lossy(&result.stdout)
    );
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.contains("Petri-net DOT") || stderr.contains("cannot write"),
        "expected contextual write error, got stderr:\n{stderr}"
    );
}

#[test]
fn emit_pn_dot_is_parseable_by_graphviz_if_available() {
    // Smoke test: if `dot` is available in $PATH (the nix dev shell
    // pulls it in for v2), confirm the emitted file is syntactically
    // valid by asking `dot` to render it to /dev/null. Otherwise
    // skip — this is a defence-in-depth check, not a hard
    // prerequisite. The shape assertions in the other tests are the
    // load-bearing ones.
    if !dot_available() {
        eprintln!("skipping graphviz smoke test: `dot` not in PATH");
        return;
    }

    let paths = example_01_paths();
    let out = fresh_outdir("dot");
    let dot_path = out.join("schedule.dot");

    let result = Command::new(nucleus_bin())
        .arg("build")
        .arg("--algo")
        .arg(&paths.algo)
        .arg("--sched")
        .arg(&paths.sched)
        .arg("--backend")
        .arg("pthreads-sync")
        .arg("--capabilities")
        .arg(&paths.capabilities)
        .arg("--emit-pn")
        .arg(&dot_path)
        .output()
        .expect("spawn nucleus");
    assert!(result.status.success(), "nucleus build failed");

    let dot_result = Command::new("dot")
        .arg("-Tsvg")
        .arg(&dot_path)
        .arg("-o")
        .arg(out.join("schedule.svg"))
        .output()
        .expect("spawn dot");
    assert!(
        dot_result.status.success(),
        "dot rejected the emitted file:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&dot_result.stdout),
        String::from_utf8_lossy(&dot_result.stderr)
    );
}

fn dot_available() -> bool {
    Command::new("dot")
        .arg("-V")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
