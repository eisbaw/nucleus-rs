//! Per-command wall-clock timeout (TASK-0453.01.01 AC#1) — now a thin
//! adapter over the SHARED implementation.
//!
//! The kill-group timeout machinery was lifted into
//! `test_common::proc_timeout` (TASK-0466) so this binary and the
//! curated `just e2e` harness share ONE implementation. This module is
//! now just the diff-fuzz-specific facade: the documented env-var name
//! (`DIFF_FUZZ_TIMEOUT_SECS`) and default, plus re-exports of the shared
//! `Timed` / `run_timed` types the per-program orchestrator
//! (`backend.rs`, `main.rs`) calls. The process-control logic — own
//! process group, deadline poll, kill-group-THEN-drain — lives in one
//! place; see `test_common::proc_timeout` for the rationale (notably the
//! drain-after-kill pipe-deadlock note).

// Re-export the shared types so the diff-fuzz call sites keep using
// `exec::run_timed` / `exec::Timed` unchanged after the lift.
pub(crate) use test_common::proc_timeout::{run_timed, Timed};

/// Default per-command wall-clock budget. A COLD `cargo build --release`
/// of a generated project (the dominant cost) can legitimately take tens
/// of seconds on a loaded CI box, and the 7 backends contend for the
/// build-directory lock, so the budget is generous on purpose: it must
/// only catch a genuine HANG (minutes-to-forever), never a slow-but-live
/// build. Override with `DIFF_FUZZ_TIMEOUT_SECS`.
pub(crate) const DEFAULT_TIMEOUT_SECS: u64 = 600;

/// Env knob name for the per-command timeout, documented in `--help`.
pub(crate) const TIMEOUT_ENV: &str = "DIFF_FUZZ_TIMEOUT_SECS";

/// Resolve the per-command timeout from the environment, falling back to
/// [`DEFAULT_TIMEOUT_SECS`]. A malformed / zero value is rejected loudly
/// (fail-fast) rather than silently coerced — a typo'd budget that
/// silently became "no timeout" would re-open the exact hang hole the
/// shared module closes. Delegates to the shared resolver so the
/// fail-fast parse/zero-reject behaviour is identical to the curated
/// harness's.
pub(crate) fn resolve_timeout() -> Result<std::time::Duration, String> {
    test_common::proc_timeout::resolve_timeout(TIMEOUT_ENV, DEFAULT_TIMEOUT_SECS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // The deep behavioural coverage (completes/captures/hang-killed/
    // group-kill-reaps) lives next to the implementation in
    // `test_common::proc_timeout`. Here we only pin the diff-fuzz facade:
    // the env name + default flow through the shared resolver.

    #[test]
    fn facade_default_budget_is_positive() {
        // No env set for `DIFF_FUZZ_TIMEOUT_SECS` in this hermetic test:
        // the shared resolver returns the diff-fuzz default.
        let d = resolve_timeout().expect("default resolves");
        assert_eq!(d.as_secs(), DEFAULT_TIMEOUT_SECS);
    }

    #[test]
    fn facade_run_timed_round_trips_completed() {
        let c = std::process::Command::new("true");
        let r = run_timed(c, Duration::from_secs(5)).expect("run");
        assert!(matches!(r, Timed::Completed(_)));
    }
}
