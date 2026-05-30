//! Integration test for TASK-0371: pin the driver-level rejection of a
//! `partition=workers` schedule whose worker count EXCEEDS the
//! partitioned-dim length (the empty-band / `InsufficientWork` reject).
//!
//! Background (cycle-214 architect P3, from the TASK-0367 review). The
//! empty-band reject — worker count > partitioned-dim length ->
//! `PartitionError::InsufficientWork`, driver exit 1, NO panic — is
//! pinned at the PASS level (`compute_partition_bands` + `map_band_error`
//! unit tests in `passes/common.rs` + `passes/partition_workers.rs`) and
//! was verified ONCE manually at the driver level. But there was no
//! DRIVER/e2e negative regression test, so if the driver's error mapping
//! (`apply_partition_workers(...).map_err(...)?` in
//! `nucleus/driver/src/main.rs`; grep `partition-workers error` for the
//! current site — symbol grep, not line number, so this docstring does
//! not rot on the next refactor) later swallowed or panicked on the
//! error, no test would catch it.
//!
//! This subprocess test exercises the REAL binary end to end: it builds a
//! 07-matmul `partition=workers` schedule that over-subscribes the outer
//! `i` axis (N=16 rows, 17 compute workers -> at least one worker gets
//! zero rows -> `InsufficientWork { len: 16, workers: 17 }`), runs
//! `nucleus build`, and asserts a non-zero process exit AND the
//! variant-specific typed-error text AND the absence of a panic marker.
//! Dropping or weakening the driver error-mapping fails the gate LOUD.
//!
//! The schedule is INVALID by construction, so it must NOT live under
//! `nuc-nucleus/examples/` (that tree is enumerated by other tooling and
//! every committed schedule is expected to build). It is written to a
//! fresh tempdir at test time. `--algo`/`--kernels` point at the REAL
//! 07-matmul example files (the driver resolves the algorithm from
//! `--algo`, not from the schedule's `schedule for "..."` header), so
//! there is no stale copy of the example to drift.
//!
//! Subprocess pattern mirrors `nucleus/driver/tests/task0048_05_shim_rejection.rs`.

use std::path::PathBuf;
use std::process::Command;

/// Walk up from `CARGO_MANIFEST_DIR` until we find the repo root.
/// Same idiom as `task0048_05_shim_rejection.rs` / `cli_reuse_strict.rs`.
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

fn fresh_outdir(tag: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("nucleus-{}-{}", tag, std::process::id()));
    if p.exists() {
        let _ = std::fs::remove_dir_all(&p);
    }
    std::fs::create_dir_all(&p).expect("create tempdir");
    p
}

/// AC#1: a `partition=workers` schedule with worker count > partitioned-dim
/// length must produce the typed `InsufficientWork` error and a failure
/// exit — NOT a panic, NOT a silent fall-through that emits a degenerate
/// zero-row-band project.
#[test]
fn oversubscribed_partition_workers_rejects_typed_nonzero_no_panic() {
    let matmul = repo_root()
        .join("nuc-nucleus")
        .join("examples")
        .join("07-matmul");
    let algo = matmul.join("prog.algo.nuc");
    let kernels = matmul.join("kernels.rs");
    assert!(
        algo.exists() && kernels.exists(),
        "07-matmul example files must exist for this test; algo={algo:?} kernels={kernels:?}"
    );

    let out = fresh_outdir("partition-insufficient-work");

    // 07-matmul is N=16. Partition the outer `i` axis (16 rows) across 17
    // compute workers w0..w16 => base = 16/17 = 0 => at least one worker
    // gets zero rows => `compute_partition_bands` returns
    // PartitionBandError::InsufficientWork, mapped to
    // PartitionError::InsufficientWork { var: i, lo: 0, hi: 16, workers: 17 }.
    // The `schedule for` header is cosmetic (driver uses --algo); we still
    // point it at the real algo with an absolute path so it resolves under
    // any future header-consulting refactor.
    let mut workers = String::from("host");
    let mut compute = String::new();
    for w in 0..17 {
        workers.push_str(&format!(", w{w}"));
        if w > 0 {
            compute.push_str(", ");
        }
        compute.push_str(&format!("w{w}"));
    }
    let sched_src = format!(
        "schedule for \"{algo}\" {{\n\
         \x20\x20\x20\x20workers = {{ {workers} }};\n\n\
         \x20\x20\x20\x20place load_a on host;\n\
         \x20\x20\x20\x20place load_b on host;\n\
         \x20\x20\x20\x20place save_c on host;\n\
         \x20\x20\x20\x20place madd   on {{ {compute} }};\n\n\
         \x20\x20\x20\x20loop i : partition=workers;\n\n\
         \x20\x20\x20\x20transfer a : sync;\n\
         \x20\x20\x20\x20transfer b : sync;\n\
         \x20\x20\x20\x20transfer c : sync;\n\
         }}\n",
        algo = algo.display(),
    );
    let sched = out.join("oversubscribed.sched.nuc");
    std::fs::write(&sched, sched_src).expect("write over-subscribed schedule");

    let result = Command::new(nucleus_bin())
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
        .output()
        .expect("spawn nucleus");

    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        !result.status.success(),
        "expected an over-subscribed partition=workers schedule (17 workers > 16 rows) to \
         FAIL, but the build succeeded (empty-band silent-fall-through regression).\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    // The InsufficientWork Display text (passes/partition_workers.rs), as
    // surfaced through the driver's `partition-workers error: {e}` wrap.
    assert!(
        combined.contains("strictly less than"),
        "expected the InsufficientWork diagnostic ('... strictly less than N workers ...') in \
         CLI output.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        combined.contains("17 workers"),
        "expected the diagnostic to name the offending worker count (17).\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        combined.contains("cannot give every worker at least one row"),
        "expected the InsufficientWork remainder-policy explanation.\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    // Must be a typed error, not a panic (PRD fail-fast rule + the
    // panic-not-diagnostic recurring defect class).
    assert!(
        !combined.contains("panicked"),
        "rejection must be a typed error, not a panic.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    let _ = std::fs::remove_dir_all(&out);
}
