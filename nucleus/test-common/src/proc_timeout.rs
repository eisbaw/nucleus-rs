//! Per-command wall-clock timeout with process-GROUP teardown.
//!
//! # Why this exists (single source of truth)
//!
//! Every build / run a test or e2e harness spawns can in principle
//! HANG: a backend deadlock is exactly the failure class the
//! compile-time soundness gate targets, and the repo has a documented
//! history of one — an untimeouted generated mp-tcp pingpong pair wedged
//! ~10.5h at 0% CPU in the connect/accept phase and cost a full night
//! (TASK-0461). Without a timeout a hung child reads as "still running",
//! not "FAIL", so the harness stalls instead of failing loud. This
//! module converts a hang into a reported, reproducing FAILURE with a
//! diagnostic tail, honouring the honest-failure discipline.
//!
//! It was lifted out of the diff-fuzz binary's private `exec.rs`
//! (TASK-0453.01.01) so that BOTH the generative fuzz harness AND the
//! curated `just e2e` harness (`nucleus/e2e/src/run.rs`,
//! `determinism.rs`) AND the backend unit-test runners (the mp-tcp
//! pingpong tests) consume ONE implementation (TASK-0466). The lifted
//! code is pure process-control infrastructure — it carries NO
//! system-under-test semantics — so sharing it does NOT couple the
//! differential oracle to the thing it validates (the build/run *flow*
//! stays deliberately duplicated in diff-fuzz; only the kill-group
//! primitive is shared).
//!
//! Scope note (kept honest for the thesis threats section): the
//! soundness gate is the real LIVENESS guard. This timeout is a
//! value-correctness instrument's safety net — it guarantees the harness
//! terminates and reports, it does NOT itself prove deadlock-freedom.
//!
//! # Mechanism
//!
//! Pure std, no extra deps. The child is spawned into its OWN process
//! group (`process_group(0)`), so the message-passing backends'
//! `bash run.sh` launcher — which forks several worker processes — can be
//! torn down as a GROUP on timeout rather than leaving orphaned workers
//! holding the scratch tree (and ports) open. We poll `try_wait()` on a
//! short cadence until the child exits or the deadline passes; on
//! deadline we SIGKILL the whole group, reap the leader, and return a
//! typed `Timeout`.

use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// Resolve a per-command timeout from the named environment variable,
/// falling back to `default_secs`. A malformed / zero value is rejected
/// loudly (fail-fast) rather than silently coerced — a typo'd budget
/// that silently became "no timeout" would re-open the exact hang hole
/// this module closes.
///
/// `env_name` is the variable each harness documents in its `--help`
/// (e.g. `DIFF_FUZZ_TIMEOUT_SECS` for the fuzzer, `NUC_E2E_TIMEOUT_SECS`
/// for the curated harness). Keeping the knob name a parameter — rather
/// than hard-coding one — is what lets the two harnesses share this
/// resolver while presenting their own documented env var.
pub fn resolve_timeout(env_name: &str, default_secs: u64) -> Result<Duration, String> {
    match std::env::var(env_name) {
        Err(_) => Ok(Duration::from_secs(default_secs)),
        Ok(raw) => {
            let secs: u64 = raw
                .trim()
                .parse()
                .map_err(|e| format!("{env_name}={raw:?} is not a u64 seconds value: {e}"))?;
            if secs == 0 {
                return Err(format!(
                    "{env_name}=0 would disable the timeout; refusing (use a positive budget)"
                ));
            }
            Ok(Duration::from_secs(secs))
        }
    }
}

/// Outcome of a timed command: either it completed (with its `Output`) or
/// it was killed for exceeding the budget.
pub enum Timed {
    Completed(Output),
    /// Hit the wall-clock budget; the child group was SIGKILL'd. Carries
    /// whatever stdout/stderr had been captured before the kill, plus the
    /// budget, so the failure report can show the last output and the
    /// limit it tripped.
    Timeout {
        elapsed: Duration,
        budget: Duration,
        partial_stdout: Vec<u8>,
        partial_stderr: Vec<u8>,
    },
}

/// Run `cmd` with a wall-clock `budget`. On expiry the child's entire
/// process group is SIGKILL'd and `Timed::Timeout` is returned; otherwise
/// `Timed::Completed` with the captured output.
///
/// stdout/stderr are captured to pipes. On timeout we still drain the
/// pipes best-effort so a partial diagnostic survives. Returns `Err` only
/// for spawn/io faults that are not the child's own exit status.
pub fn run_timed(mut cmd: Command, budget: Duration) -> Result<Timed, String> {
    use std::os::unix::process::CommandExt as _;
    // Own process group so a `bash run.sh` that forks workers can be torn
    // down as a unit. `process_group(0)` => the child becomes the leader
    // of a new group whose pgid == child pid (stable since Rust 1.64).
    cmd.process_group(0);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| format!("spawn: {e}"))?;
    let pid = child.id() as i32;
    let start = Instant::now();

    // Poll cadence: short enough that the reported `elapsed` is tight,
    // long enough not to burn a core spinning. The build phase dominates
    // wall-clock by orders of magnitude, so 50ms granularity is free.
    let poll = Duration::from_millis(50);
    loop {
        match child.try_wait().map_err(|e| format!("try_wait: {e}"))? {
            Some(_status) => {
                // Completed within budget — collect the full output. Use
                // `wait_with_output` so the captured pipes are drained.
                let out = child
                    .wait_with_output()
                    .map_err(|e| format!("wait_with_output: {e}"))?;
                return Ok(Timed::Completed(out));
            }
            None => {
                if start.elapsed() >= budget {
                    let elapsed = start.elapsed();
                    // Kill the WHOLE GROUP first, THEN drain. Draining
                    // before the kill would deadlock: `read_to_end` blocks
                    // until every writer of the pipe is closed, and a
                    // forked grandchild (e.g. the `bash run.sh` workers)
                    // still holds the write end open while it runs. Killing
                    // the group closes all those write ends, so the drain
                    // then reads buffered bytes and returns EOF promptly.
                    kill_group(pid);
                    let partial_stdout = drain(child.stdout.take());
                    let partial_stderr = drain(child.stderr.take());
                    // Reap the leader so it does not linger as a zombie.
                    let _ = child.wait();
                    return Ok(Timed::Timeout {
                        elapsed,
                        budget,
                        partial_stdout,
                        partial_stderr,
                    });
                }
                std::thread::sleep(poll);
            }
        }
    }
}

/// Best-effort, non-blocking-ish drain of a captured pipe. We are about to
/// SIGKILL the writer, so a short blocking read of whatever is buffered is
/// acceptable; failures are swallowed (the diagnostic is best-effort).
fn drain(pipe: Option<impl Read>) -> Vec<u8> {
    let mut buf = Vec::new();
    if let Some(mut p) = pipe {
        let _ = p.read_to_end(&mut buf);
    }
    buf
}

/// SIGKILL an entire process group by negative pgid. We set the group up
/// in [`run_timed`] so `pgid == pid`; killing `-pid` reaches the leader
/// and every worker it forked. Errors are ignored — the child may have
/// exited between the deadline check and here (a benign race).
fn kill_group(pid: i32) {
    // We avoid an FFI dependency (no `libc` dep, no `unsafe`) by shelling
    // out to `kill(1)`, which is always present on the Linux dev/CI host
    // these harnesses target. A signal to a whole process group is
    // addressed with the NEGATIVE pgid; since `run_timed` makes the child
    // its own group leader (`pgid == pid`), `kill -KILL -<pid>` reaches
    // the leader and every worker it forked.
    let _ = Command::new("kill")
        .arg("-KILL")
        .arg(format!("-{pid}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    // Also target the leader directly in case the group teardown raced.
    let _ = Command::new("kill")
        .arg("-KILL")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Convenience wrapper for unit-test call sites (e.g. the mp-tcp pingpong
/// runners): run `cmd` under `budget`, panicking with a `label`-tagged,
/// tail-bearing message on timeout so a wedged generated pair FAILS the
/// test in minutes instead of stalling the gate overnight (TASK-0461
/// AC#2). On completion it returns the captured `Output` unchanged, so
/// callers keep their existing `.status` / `.stdout` / `.stderr`
/// assertions.
///
/// # Panics
///
/// Panics if the command times out (the wedged-pair signature) or if the
/// spawn itself faults — both are honest test failures, not silent
/// masking.
pub fn run_or_timeout(cmd: Command, budget: Duration, label: &str) -> Output {
    match run_timed(cmd, budget).unwrap_or_else(|e| panic!("{label}: spawn failed: {e}")) {
        Timed::Completed(out) => out,
        Timed::Timeout {
            elapsed,
            budget,
            partial_stdout,
            partial_stderr,
        } => {
            let tail = |b: &[u8], n: usize| -> String {
                let s = String::from_utf8_lossy(b);
                let v: Vec<&str> = s.lines().collect();
                v[v.len().saturating_sub(n)..].join("\n")
            };
            panic!(
                "{label}: TIMED OUT after {:.1}s (budget {:.0}s) — treated as a HANG/FAIL \
                 (the wedged-generated-pair signature; TASK-0461). \
                 Last stdout:\n{}\nLast stderr:\n{}",
                elapsed.as_secs_f64(),
                budget.as_secs_f64(),
                tail(&partial_stdout, 12),
                tail(&partial_stderr, 12),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completes_within_budget() {
        let c = Command::new("true");
        let r = run_timed(c, Duration::from_secs(5)).expect("run");
        match r {
            Timed::Completed(out) => assert!(out.status.success()),
            Timed::Timeout { .. } => panic!("`true` should not time out"),
        }
    }

    #[test]
    fn captures_stdout() {
        let mut c = Command::new("sh");
        c.arg("-c").arg("printf hello");
        let r = run_timed(c, Duration::from_secs(5)).expect("run");
        match r {
            Timed::Completed(out) => assert_eq!(out.stdout, b"hello"),
            Timed::Timeout { .. } => panic!("should complete"),
        }
    }

    #[test]
    fn hang_is_killed_and_reported() {
        // `sleep 1000` is a stand-in for a deadlocked backend. The budget
        // is sub-second so the test is fast; the kill must reap it.
        let mut c = Command::new("sleep");
        c.arg("1000");
        let start = Instant::now();
        let r = run_timed(c, Duration::from_millis(300)).expect("run");
        match r {
            Timed::Timeout {
                elapsed, budget, ..
            } => {
                assert!(elapsed >= budget);
                // We must have returned promptly after the budget, not
                // waited for the full 1000s sleep.
                assert!(start.elapsed() < Duration::from_secs(5));
            }
            Timed::Completed(_) => panic!("`sleep 1000` must time out under a 300ms budget"),
        }
    }

    #[test]
    fn group_kill_reaps_forked_children() {
        // A shell that forks a long sleeper then itself sleeps. The whole
        // group must die; we assert the call returns promptly (the group
        // kill reached the forked sleeper, not just the shell).
        let mut c = Command::new("sh");
        c.arg("-c").arg("sleep 1000 & sleep 1000");
        let start = Instant::now();
        let r = run_timed(c, Duration::from_millis(300)).expect("run");
        assert!(matches!(r, Timed::Timeout { .. }));
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn resolve_timeout_default_is_positive() {
        // Use a name no other test sets so this stays hermetic under the
        // process-global env even with `cargo test` parallelism.
        let d = resolve_timeout("NUC_TEST_COMMON_TIMEOUT_UNUSED_XYZ", 600).expect("default");
        assert_eq!(d.as_secs(), 600);
    }

    #[test]
    fn run_or_timeout_panics_on_hang() {
        // Build the `Command` INSIDE the closure: `Command` is not
        // `UnwindSafe`, so closing over one would not satisfy
        // `catch_unwind`'s bound.
        let r = std::panic::catch_unwind(|| {
            let mut c = Command::new("sleep");
            c.arg("1000");
            run_or_timeout(c, Duration::from_millis(200), "hang-probe")
        });
        assert!(r.is_err(), "run_or_timeout must panic on a hang");
    }

    #[test]
    fn run_or_timeout_returns_output_on_success() {
        let mut c = Command::new("sh");
        c.arg("-c").arg("printf ok");
        let out = run_or_timeout(c, Duration::from_secs(5), "ok-probe");
        assert!(out.status.success());
        assert_eq!(out.stdout, b"ok");
    }
}
