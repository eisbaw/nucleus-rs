//! Zero-dependency, env-gated compiler trace facility.
//!
//! # Why not `log` + `env_logger` / `tracing`? (TASK-0154 AC#1)
//!
//! The compiler crate deliberately carries only four dependencies
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
//! - `NUC_XBACKEND_NEGATIVE` — `mp-tcp-bufsync/src/lib.rs:1154`
//!   (same shape).
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

use std::cell::RefCell;

thread_local! {
    /// Test-only capture sink. When `Some`, `nuc_trace!` appends the
    /// formatted line here instead of stderr — *regardless* of the
    /// `NUC_TRACE` env var, so a test can assert on emitted lines
    /// deterministically without racing the process environment or
    /// scraping the real stderr. Production code never sets this.
    pub(crate) static TRACE_SINK: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

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

/// Internal sink for a formatted trace line. Routes to the test
/// capture buffer if one is installed on this thread, otherwise to
/// stderr — and only if `NUC_TRACE` is enabled. Keeping the routing
/// here (not in the macro body) keeps the macro expansion tiny.
#[doc(hidden)]
pub fn emit(line: std::fmt::Arguments<'_>) {
    let captured = TRACE_SINK.with(|s| {
        if let Some(buf) = s.borrow_mut().as_mut() {
            buf.push(format!("{line}"));
            true
        } else {
            false
        }
    });
    if captured {
        return;
    }
    if trace_enabled() {
        eprintln!("nucleus: trace: {line}");
    }
}

/// Emit a compiler trace line. **No-op unless `NUC_TRACE` is set** (or
/// a test sink is installed). Arguments are not formatted on the
/// disabled path.
///
/// ```ignore
/// nuc_trace!("transfer_inject: deferred {} seq {}", sym, seq);
/// ```
#[macro_export]
macro_rules! nuc_trace {
    ($($arg:tt)*) => {
        // Cheap guard first: when a test sink is active we must still
        // capture even if NUC_TRACE is unset, so check both. The env
        // lookup is trivial and off the hot path.
        if $crate::trace::trace_enabled()
            || $crate::trace::test_sink_active()
        {
            $crate::trace::emit(format_args!($($arg)*));
        }
    };
}

/// Whether a test capture sink is installed on the current thread.
/// Exposed (doc-hidden) only so the `nuc_trace!` macro can short-
/// circuit correctly under test without an env var.
#[doc(hidden)]
pub fn test_sink_active() -> bool {
    TRACE_SINK.with(|s| s.borrow().is_some())
}

/// RAII guard that installs a thread-local capture sink for the
/// duration of a test and yields the collected lines on drop via
/// [`TraceCapture::lines`]. Test-only helper.
#[doc(hidden)]
pub struct TraceCapture;

impl TraceCapture {
    /// Install a fresh capture buffer on this thread.
    pub fn start() -> Self {
        TRACE_SINK.with(|s| *s.borrow_mut() = Some(Vec::new()));
        TraceCapture
    }

    /// Take the captured lines so far (clears the buffer).
    pub fn lines(&self) -> Vec<String> {
        TRACE_SINK.with(|s| {
            s.borrow_mut()
                .as_mut()
                .map(std::mem::take)
                .unwrap_or_default()
        })
    }
}

impl Drop for TraceCapture {
    fn drop(&mut self) {
        TRACE_SINK.with(|s| *s.borrow_mut() = None);
    }
}
