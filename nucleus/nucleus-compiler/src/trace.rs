//! Zero-dependency, env-gated compiler trace facility.
//!
//! # Decision (TASK-0280, cycle 108): KEEP + preserved-as-convention
//!
//! TASK-0280 audited this facility after TASK-0267 (cycle 101) removed
//! `transfer_inject::trace_block_deferral`, which had been the only
//! in-source consumer at the time. As of cycle 108 the production
//! consumer set is:
//!
//! - `nucleus/driver/src/main.rs:399` — emits halo_inference
//!   advisory errors (PartitionAware mode; non-fatal when the
//!   affected iv has no `partition=` directive in scope, so the
//!   transfer_inject halo consumer would not fire on it).
//!
//! Decision: preserve the facility per PRD §12 / CLAUDE.md
//! `decision-0001` (zero-dep, env-gated, do NOT add `log`/`tracing`).
//! It is the established convention for "advisory diagnostics on a
//! pass that does not fail-loud". Future passes (sync_inject,
//! petri_to_events, partition_*) that gain advisory-error paths
//! follow the same pattern — calling `nuc_trace!(...)` from the
//! driver where the per-pass advisory bucket is collected.
//!
//! Cycle 109 (TASK-0285) pruned the unused `TraceCapture` test-side
//! sink + `TRACE_SINK` thread-local + `test_sink_active()` helper:
//! they had no in-source consumers (zero tests in the workspace
//! exercised them), so they were dead code. If a future test needs to
//! capture trace lines without scraping stderr, the
//! `RefCell<Option<Vec<String>>>` thread-local + RAII guard pattern
//! is straightforward to re-introduce — but for now `nuc_trace!` is
//! a stderr-only emit gated on `NUC_TRACE`. The macro body shrank
//! from a two-condition guard (`trace_enabled() || test_sink_active()`)
//! to a single `trace_enabled()` check.
//!
//! # Why not `log` + `env_logger` / `tracing`? (TASK-0154 AC#1)
//!
//! The nucleus-compiler crate deliberately carries only four dependencies
//! (chumsky / syn / quote / serde), each pulling its weight, with the
//! MSRV pinned in the Nix flake (PRD §12.1) and a hard no-spam ethos
//! (PRD §12.3; `~/.claude/CLAUDE.md`). PRD §12 is explicit: "Three
//! tools, each doing one thing." A logging *facade* + a backend
//! implementation for a literal handful of deferral-trace lines is two
//! more crates (plus their transitive trees, which `env_logger` in
//! particular drags in — `regex`, `aho-corasick`, …), a second MSRV
//! surface that can drift past the flake pin, and a global mutable
//! logger init that fights the "no hidden machinery" principle (§12.2).
//! That cost is not proportionate to the need.
//!
//! Instead this mirrors the discipline the codebase **already**
//! standardised on for behaviour switches:
//!
//! - `NUC_NONDET_TEST` — `e2e/src/main.rs` (`maybe_perturb_for_nondet_test`)
//!   (`std::env::var(..).as_deref() == Ok("1")`, loud stderr banner;
//!   harness-side post-emit perturbation, not on the codegen path —
//!   relocated there in TASK-0157).
//! - `NUC_XBACKEND_NEGATIVE` — `e2e/src/main.rs`
//!   (`maybe_corrupt_wire_for_xbackend`) (same shape; harness-side
//!   post-emit corruption of the mp-tcp `wire.rs`, not on the codegen
//!   path — relocated there in TASK-0183, parallel to TASK-0157).
//!
//! A tiny in-house `nuc_trace!` macro, **silent unless `NUC_TRACE` is
//! set**, writing to stderr, is the consistent choice — not a novel
//! one. It adds zero dependencies and zero MSRV surface.
//!
//! ## Env-gate vs `cfg!(debug_assertions)`
//!
//! `cfg!(debug_assertions)` was rejected: it is a *compile-time* knob.
//! Turning trace on/off would require a rebuild, the release binary
//! could never emit it, and it would diverge from the existing
//! `NUC_*` precedent (all runtime-gated). A runtime env gate is
//! selectable without recompiling, works on the shipped binary, and is
//! exactly the discipline already in the tree. The cost — a cheap
//! `env::var` lookup on the trace path — is irrelevant: the trace path
//! is off the hot path and only taken at structural deferral points.
//!
//! ## Default path is byte-silent (determinism / e2e safety)
//!
//! `NUC_TRACE` unset ⇒ the macro evaluates its guard and returns
//! without touching stderr or stdout and without formatting its
//! arguments. Generated code and all normal `nucleus build` / e2e
//! output are byte-identical to before this facility existed. This is
//! a hard requirement: any unconditional output would break
//! `just determinism-check` and the e2e snapshot.

/// Is tracing enabled for this run? True iff `NUC_TRACE` is set to a
/// non-empty value. The exact-`"1"` check used by `NUC_NONDET_TEST`
/// would be needlessly strict for a diagnostic (operators expect
/// `NUC_TRACE=1` *or* `NUC_TRACE=transfer_inject` etc. to work); any
/// non-empty value enables it. Still off-by-default and value-gated.
#[doc(hidden)]
pub fn trace_enabled() -> bool {
    std::env::var_os("NUC_TRACE")
        .map(|v| !v.is_empty())
        .unwrap_or(false)
}

/// Internal sink for a formatted trace line. Writes to stderr when
/// `NUC_TRACE` is enabled. Keeping the routing here (not in the macro
/// body) keeps the macro expansion tiny.
#[doc(hidden)]
pub fn emit(line: std::fmt::Arguments<'_>) {
    if trace_enabled() {
        eprintln!("nucleus: trace: {line}");
    }
}

/// Emit a compiler trace line. **No-op unless `NUC_TRACE` is set.**
/// Arguments are not formatted on the disabled path.
///
/// ```ignore
/// // Driver halo_inference advisory pattern (the live cycle-108
/// // consumer; see `nucleus/driver/src/main.rs:399`):
/// nuc_trace!(
///     "halo_inference: advisory (no `partition=` directive in scope, transfer_inject \
///      halo consumer will not fire on the affected iv — lowering proceeds): {e}"
/// );
/// ```
#[macro_export]
macro_rules! nuc_trace {
    ($($arg:tt)*) => {
        if $crate::trace::trace_enabled() {
            $crate::trace::emit(format_args!($($arg)*));
        }
    };
}
