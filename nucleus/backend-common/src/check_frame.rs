//! Shared `check loop V : on_violation={panic,log,count}` emit
//! templates (TASK-0052.02 / .04 / .05; cross-backend deduplication
//! TASK-0222, moved into backend-common by TASK-0244).
//!
//! The four templates below were previously verbatim-duplicated between
//! pthreads-sync (single-worker + multi-worker codegen) and
//! mp-tcp-bufsync (8 inline sites total: 2 backends × 4 templates).
//! pthreads-async's multi-worker arm (TASK-0228 Wave B-2) became the
//! third tier-1 consumer. The "two readers can hold it in their head"
//! heuristic no longer holds; extracting the templates into `pub fn`
//! helpers means a single edit propagates to every backend by
//! construction (drift detection becomes structural prevention, not
//! test-as-tripwire).
//!
//! Each helper takes `out: &mut String` and writes ONE emit unit
//! (one Rust statement / declaration). The caller owns indentation,
//! loop iteration, and the surrounding context — these helpers are
//! the smallest unit of shared template, not a wrapper for the whole
//! codegen flow.

use std::fmt::Write as _;

use nucleus_compiler::event::{Event, ViolationKind};

/// One Count check loop, materialised for codegen. Public so every
/// backend (pthreads-sync, pthreads-async multi-worker, mp-tcp-bufsync)
/// can reuse the SAME collector and emit the SAME shape — single
/// implementation, no drift, the cross-backend differential property
/// holds.
#[derive(Debug, Clone)]
pub struct CountCheckLoop {
    /// Sanitized identifier suffix — appears in the static name
    /// `NUC_CHECK_COUNT_<ident>` and the guard local
    /// `_nuc_check_reporter_<ident>`.
    pub ident: String,
    /// Original loop variable name (carried verbatim into the
    /// stderr summary so the user sees the directive they wrote).
    pub loop_var: String,
    /// Threshold in nanoseconds (post-unit-normalisation, same as
    /// `CheckFrame::latency_max_ns`).
    pub latency_max_ns: u64,
}

/// Replace any non-`[A-Za-z0-9_]` byte with `_`; if the resulting
/// first byte is a digit, prefix `_`. Pure-ASCII; idempotent. Two
/// loop_vars that differ only outside the alphabet (e.g. `a-1` vs
/// `a_1`) collide post-sanitization — but the parser would already
/// have rejected the first form, so this is an unreachable contract
/// position in practice; documenting it here rather than guarding it
/// with a runtime check.
pub fn sanitize_loop_var(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() {
        s.push('_');
    }
    if s.as_bytes()[0].is_ascii_digit() {
        s.insert(0, '_');
    }
    s
}

/// Walk `events` recursively; collect every Count check_frame in
/// EventList order. Deterministic order is the EventList walk
/// order (which is itself deterministic across builds — same
/// guarantee the rest of codegen relies on).
pub fn collect_count_check_frames(events: &[Event]) -> Vec<CountCheckLoop> {
    let mut out = Vec::new();
    fn walk(events: &[Event], out: &mut Vec<CountCheckLoop>) {
        for e in events {
            if let Event::Loop {
                body, check_frame, ..
            } = e
            {
                if let Some(frame) = check_frame {
                    if matches!(frame.on_violation, ViolationKind::Count) {
                        out.push(CountCheckLoop {
                            ident: sanitize_loop_var(&frame.loop_var),
                            loop_var: frame.loop_var.clone(),
                            latency_max_ns: frame.latency_max_ns,
                        });
                    }
                }
                walk(body, out);
            }
        }
    }
    walk(events, &mut out);
    out
}

/// Emit the file-scope Drop-guard struct + its `Drop` impl. Called
/// from `render_main_rs` only when at least one Count check loop is
/// present. The summary message is gated by `n > 0` so a clean run
/// (zero violations) prints NOTHING to stderr — keeping stderr quiet
/// on the happy path is what makes the cross-backend differential
/// indifferent to Count's presence in the schedule.
pub fn emit_count_reporter_struct(out: &mut String) {
    writeln!(
        out,
        "// TASK-0052.04: per-`check loop` Count summary guard.\n\
         struct NucCheckCountReporter {{\n\
         \x20\x20\x20\x20counter: &'static std::sync::atomic::AtomicU64,\n\
         \x20\x20\x20\x20loop_var: &'static str,\n\
         \x20\x20\x20\x20threshold_ns: u64,\n\
         }}\n\
         impl Drop for NucCheckCountReporter {{\n\
         \x20\x20\x20\x20fn drop(&mut self) {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20let n = self.counter.load(std::sync::atomic::Ordering::Relaxed);\n\
         \x20\x20\x20\x20\x20\x20\x20\x20if n > 0 {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20eprintln!(\"check loop `{{}}` violated latency_max={{}} ns: {{}} occurrence(s)\", self.loop_var, self.threshold_ns, n);\n\
         \x20\x20\x20\x20\x20\x20\x20\x20}}\n\
         \x20\x20\x20\x20}}\n\
         }}"
    )
    .ok();
}

/// Emit the file-scope `static NUC_CHECK_COUNT_<ident>: AtomicU64`
/// declaration for a Count check_loop. Two lines (a per-static
/// `#[allow(non_upper_case_globals)]` attribute + the static itself) +
/// trailing newline.
///
/// The static name embeds the source loop-var spelling (e.g. a
/// lowercase `i`/`n`, via [`sanitize_loop_var`]) so it stays greppable
/// back to the `check loop` directive. That deliberately-cased generated
/// identifier trips rustc's `non_upper_case_globals` style lint in the
/// generated crate. The embedded skeletons allow it crate-wide
/// (`#![allow(... non_upper_case_globals)]`, TASK-0048.08); the tier-1 /
/// multi-process generated `main.rs` has no such preamble, so the lint
/// is silenced here per-static (TASK-0386 — targeted, not a crate-wide
/// blanket, so other globals stay linted). Source-only: the attribute
/// does not affect the runtime output the e2e differential diffs.
///
/// Caller wraps this in a `for cf in count_frames` loop after
/// [`emit_count_reporter_struct`], and emits a blank `writeln!(out)`
/// after the loop. Mirrors the pre-extraction pthreads-sync site at
/// lib.rs:551-559 and mp-tcp-bufsync at lib.rs:463-471 (TASK-0052.04).
pub fn emit_count_static(out: &mut String, ident: &str) {
    writeln!(
        out,
        "#[allow(non_upper_case_globals)]\n\
         static NUC_CHECK_COUNT_{ident}: std::sync::atomic::AtomicU64 = \
         std::sync::atomic::AtomicU64::new(0);",
    )
    .ok();
}

/// Emit the per-Count-loop Drop guard local inside `fn main()`. A
/// five-line block: `let _nuc_check_reporter_<ident> = NucCheckCountReporter { ... };`.
///
/// Caller iterates `for cf in count_frames`, and emits a blank line
/// after the loop if `!count_frames.is_empty()`. Mirrors the
/// pre-extraction pthreads-sync site at lib.rs:572-583 and
/// mp-tcp-bufsync at lib.rs:588-600 (TASK-0052.04).
///
/// The four-space leading indent is HARDCODED to match the
/// `fn main()`-level scope all callers emit into. If a future
/// codegen path needs nested-scope emission, add a `pad: &str`
/// parameter.
pub fn emit_count_guard_local(out: &mut String, ident: &str, loop_var: &str, latency_max_ns: u64) {
    writeln!(
        out,
        "    let _nuc_check_reporter_{ident} = NucCheckCountReporter {{\n\
         \x20\x20\x20\x20\x20\x20\x20\x20counter: &NUC_CHECK_COUNT_{ident},\n\
         \x20\x20\x20\x20\x20\x20\x20\x20loop_var: \"{loop_var}\",\n\
         \x20\x20\x20\x20\x20\x20\x20\x20threshold_ns: {ns},\n\
         \x20\x20\x20\x20}};",
        ns = latency_max_ns,
    )
    .ok();
}

/// Emit the Log on-violation branch — one inline conditional that
/// runs at the bottom of an outer-loop iteration after `_check_elapsed`
/// has been computed. The branch prints to stderr (NOT stdout, so the
/// cross-backend differential on output.bin remains stable; PRD §6.3.5
/// 'Log fires sparingly').
///
/// `body_pad` is the caller's indentation string (e.g. `"        "`
/// for two nested fors). `loop_var` appears verbatim in the
/// `\`check loop \`&lt;lv&gt;\`\`` backticks of the user-visible message.
/// `latency_max_ns` appears twice: in the threshold compare and in
/// the printed line. Mirrors pre-extraction pthreads-sync at
/// lib.rs:991-997 and mp-tcp-bufsync at lib.rs:809-815 (TASK-0052.04).
pub fn emit_log_branch(out: &mut String, body_pad: &str, loop_var: &str, latency_max_ns: u64) {
    writeln!(
        out,
        "{body_pad}if _check_elapsed > {ns}_u128 {{ \
         eprintln!(\"warning: check loop `{lv}` violated latency_max={ns} ns: iteration took {{}} ns\", _check_elapsed); }}",
        ns = latency_max_ns,
        lv = loop_var,
    )
    .ok();
}

/// Emit the Count on-violation branch — one inline conditional that
/// atomically increments the file-scope `NUC_CHECK_COUNT_<id>` counter
/// (Relaxed ordering is sufficient; see pthreads-sync's pre-extraction
/// comment at lib.rs:1010-1018 for the memory-ordering rationale).
///
/// `body_pad` is the caller's indentation string. `id` is the
/// SANITIZED ident (call [`sanitize_loop_var`] before passing here —
/// this helper does NOT sanitize because the multi-worker path needs
/// a per-thread-pre-sanitised value, and the call-site already has
/// it). Mirrors pre-extraction pthreads-sync at lib.rs:1020-1026 and
/// mp-tcp-bufsync at lib.rs:827-833 (TASK-0052.04).
pub fn emit_count_branch(
    out: &mut String,
    body_pad: &str,
    sanitized_ident: &str,
    latency_max_ns: u64,
) {
    writeln!(
        out,
        "{body_pad}if _check_elapsed > {ns}_u128 {{ \
         NUC_CHECK_COUNT_{id}.fetch_add(1, std::sync::atomic::Ordering::Relaxed); }}",
        ns = latency_max_ns,
        id = sanitized_ident,
    )
    .ok();
}
