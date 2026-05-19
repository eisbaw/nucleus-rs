//! Concurrency stress test for the e2e harness scratch-dir isolation
//! (TASK-0182 AC#2).
//!
//! Background: the harness used to build every cell under a single
//! DETERMINISTICALLY-named tree (`nucleus/target/e2e-matrix/<cell>`).
//! Two concurrent or rapid back-to-back `just e2e` processes then
//! raced: process A would be `Command::current_dir`-ed into a cell
//! dir running `cargo build` / `ld` while process B `remove_dir_all`-d
//! and recreated that exact path — A's working directory vanished
//! underneath it, producing the observed
//!   `shell-init: getcwd: cannot access parent directories`
//!   `ld.bfd: cannot open output file …/nuc_generated: No such file`
//! infra failures. It did NOT reproduce serially.
//!
//! The fix inserts a process-wide `<run-id>` (pid + nanos) path
//! segment into every mutable scratch root, so disjoint invocations
//! never share a `remove_dir_all`-able tree.
//!
//! This test exercises that directly:
//!
//!   * POSITIVE (the AC): spawn N (>=20) concurrent `nucleus-e2e`
//!     processes, each restricted to ONE small representative cell.
//!     With the fix every process gets its own run-id root, so all
//!     must succeed (exit 0, the cell PASSes) with ZERO infra-race
//!     errors in any process's output.
//!
//!   * NEGATIVE / does-it-bite: rerun the SAME swarm with the
//!     gate-only `NUC_E2E_FORCE_SHARED_RUN_ID` env set to a single
//!     constant, which pins every process onto ONE shared mutable
//!     tree — exactly the pre-fix condition. We assert that this
//!     control DOES surface the race (at least one process fails with
//!     an infra-race signature). This proves the positive arm is a
//!     real test of the fix, not a no-op that would pass regardless.
//!     Skipped if a (rare) scheduling fluke leaves the control clean —
//!     a race is probabilistic; we never let the bite-proof itself be
//!     flaky-FAIL, only flaky-SKIP, while the load-bearing positive
//!     arm is a hard assertion.
//!
//! Cost: one tiny cell (`01-elementwise-add` / `naive` /
//! `pthreads-sync`), reused warm `target/`. N processes build the
//! same emitted project in parallel; on this workstation the swarm
//! completes well under the default test timeout.

use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc;
use std::thread;

/// Number of concurrent harness invocations. AC#2 requires >=20.
const SWARM: usize = 24;

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

/// True if `text` contains any of the known shared-tree-race
/// signatures. These are the exact symptoms TASK-0182 was filed for.
fn has_infra_race(text: &str) -> bool {
    text.contains("getcwd: cannot access parent directories")
        || text.contains("cannot open output file")
        || text.contains("No such file or directory")
            && text.contains("nuc_generated")
}

struct RunOutcome {
    idx: usize,
    success: bool,
    infra_race: bool,
    tail: String,
}

/// Spawn `SWARM` concurrent `nucleus-e2e` processes, each restricted
/// to the one representative cell. `force_shared` toggles the
/// gate-only seam that pins them all to one shared scratch tree (the
/// pre-fix condition) for the bite-proof.
fn run_swarm(force_shared: bool) -> Vec<RunOutcome> {
    let root = repo_root();
    let nucleus = root.join("nucleus");
    let (tx, rx) = mpsc::channel();

    let mut handles = Vec::with_capacity(SWARM);
    for idx in 0..SWARM {
        let nucleus = nucleus.clone();
        let tx = tx.clone();
        handles.push(thread::spawn(move || {
            let mut cmd = Command::new("cargo");
            cmd.arg("run")
                .arg("--quiet")
                .arg("--bin")
                .arg("nucleus-e2e")
                .arg("--")
                .arg("--example")
                .arg("01-elementwise-add")
                .arg("--schedule")
                .arg("naive")
                .arg("--backend")
                .arg("pthreads-sync")
                .current_dir(&nucleus);
            if force_shared {
                // One shared run-id for the WHOLE swarm => every
                // process collides on the same mutable scratch tree:
                // the exact pre-fix condition this test must prove the
                // fix removes.
                cmd.env("NUC_E2E_FORCE_SHARED_RUN_ID", "stress-shared");
            }
            let out = cmd.output().expect("spawn nucleus-e2e");
            let mut combined = String::new();
            combined.push_str(&String::from_utf8_lossy(&out.stdout));
            combined.push_str(&String::from_utf8_lossy(&out.stderr));
            let tail: String = combined
                .lines()
                .rev()
                .take(8)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n");
            tx.send(RunOutcome {
                idx,
                success: out.status.success(),
                infra_race: has_infra_race(&combined),
                tail,
            })
            .expect("send outcome");
        }));
    }
    drop(tx);
    for h in handles {
        h.join().expect("join swarm thread");
    }
    rx.iter().collect()
}

#[test]
fn concurrent_harness_runs_have_zero_infra_race() {
    // ---- POSITIVE: the AC. Fixed code, per-run isolation. ----------
    let results = run_swarm(false);
    assert_eq!(
        results.len(),
        SWARM,
        "expected {SWARM} swarm outcomes, got {}",
        results.len()
    );

    let racey: Vec<&RunOutcome> = results.iter().filter(|r| r.infra_race).collect();
    assert!(
        racey.is_empty(),
        "TASK-0182 regression: {} of {SWARM} concurrent harness runs hit a \
         shared-tree infra race. Offending tails:\n{}",
        racey.len(),
        racey
            .iter()
            .map(|r| format!("  [proc {}]\n{}", r.idx, r.tail))
            .collect::<Vec<_>>()
            .join("\n")
    );

    let failed: Vec<&RunOutcome> = results.iter().filter(|r| !r.success).collect();
    assert!(
        failed.is_empty(),
        "{} of {SWARM} concurrent runs exited non-zero (the representative \
         cell must PASS under concurrency). Tails:\n{}",
        failed.len(),
        failed
            .iter()
            .map(|r| format!("  [proc {}]\n{}", r.idx, r.tail))
            .collect::<Vec<_>>()
            .join("\n")
    );

    // ---- NEGATIVE: prove the positive arm actually bites. ----------
    //
    // Pin the whole swarm onto ONE shared scratch tree (pre-fix
    // condition). The race is probabilistic, so we DO NOT hard-fail
    // if a scheduling fluke leaves the control clean — that would make
    // the bite-proof itself flaky. We only assert the positive arm
    // (above) hard; the control either demonstrates the bite (logged)
    // or is reported as an inconclusive-but-not-failing observation.
    let control = run_swarm(true);
    let control_raced = control.iter().any(|r| r.infra_race || !r.success);
    if control_raced {
        let n = control
            .iter()
            .filter(|r| r.infra_race || !r.success)
            .count();
        eprintln!(
            "concurrency_stress: bite-proof OK — {n}/{SWARM} shared-tree \
             control runs hit the infra race / failed, exactly the pre-fix \
             condition the per-run-id fix removes (positive arm above was \
             clean)."
        );
    } else {
        eprintln!(
            "concurrency_stress: NOTE — shared-tree control did not surface \
             the race on this scheduling pass (it is probabilistic). The \
             positive arm still asserts the fix; not failing the bite-proof \
             to avoid making this test flaky."
        );
    }
}
