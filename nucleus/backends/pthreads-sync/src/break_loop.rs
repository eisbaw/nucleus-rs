//! `for..until` early-exit (epic S5) codegen helpers for the
//! single-worker sequential backend: break-generation capture +
//! runtime final-read (TASK-0341.02.01.05.02) and cap-hit-not-converged
//! observability (TASK-0341.02.01.05.03).
//!
//! These are the post-S4 additions on top of the break EMIT itself
//! (the `if <cond> { __nuc_break_gen = t; break; }` line, emitted in the
//! `Event::Loop` arm of `render_event` in `lib.rs`). The pieces here:
//!
//!   - [`collect_break_loop_info`] pre-scans the top-level EventList for
//!     the `for..until` loop and records the cap + the arrays it writes;
//!   - [`rewrite_final_reads`] STRUCTURALLY rewrites the post-loop
//!     extraction reads of those arrays from the compile-time cap slice
//!     to the runtime `__nuc_final_gen` (NOT a textual replace — the
//!     `feedback-textual-replace-codegen-unsafe` foot-gun);
//!   - [`emit_cap_hit_resolution`] emits the cap-hit stderr diagnostic +
//!     the `__nuc_final_gen` resolution after the loop.
//!
//! Extracted from `lib.rs` (cycle 262) to keep that file under the
//! mega-file fence; the break machinery is a cohesive unit.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use backend_common::render::{render_bool_expr, EmitError, RenderCtx};
use nucleus_compiler::algo::IrExpr;
use nucleus_compiler::event::{ArgBinding, DataId, DataSlice, Event};
use nucleus_compiler::sidecar::NameSidecar;

use crate::walk_fire_outputs;

/// Render the TOP-LEVEL event stream of the single-worker `fn main`,
/// handling the `for..until` early-exit machinery in one place
/// (extracted from `render_main_rs` in cycle-262 per architect P2-1 so
/// the cohesive break logic does not grow `lib.rs`, which is already at
/// the mega-file fence — TASK-0437).
///
/// When the stream carries no break loop ([`collect_break_loop_info`]
/// returns `None`) this is exactly `render_events(.., 1, ..)` — the
/// byte-identical plain-program path. When it DOES carry one, we
/// declare `__nuc_break_gen` (sentinel -1) before the loop; STRUCTURALLY
/// rewrite post-loop reads of break-written arrays from `[CAP]` to
/// `[__nuc_final_gen]` ([`rewrite_final_reads`]); then render every
/// top-level event, splicing the cap-hit-not-converged diagnostic +
/// `__nuc_final_gen` resolution ([`emit_cap_hit_resolution`]) immediately
/// after the break loop so they precede the (rewritten) extraction reads.
/// The break EMIT itself (the capture-before-break line) is
/// [`emit_break_check`], called from the `Event::Loop` arm of
/// `render_event` in `lib.rs`.
pub(crate) fn render_top_level_events(
    events: &[Event],
    out: &mut String,
    ctx: &RenderCtx<'_>,
    sidecar: &NameSidecar,
) -> Result<(), EmitError> {
    match collect_break_loop_info(events) {
        None => crate::render_events(events, out, 1, ctx),
        Some(info) => {
            let mut rewritten: Vec<Event> = events.to_vec();
            rewrite_final_reads(&mut rewritten, &info, sidecar, false);
            // `__nuc_break_gen`: -1 sentinel = did-not-converge. An i64 to
            // match the loop var's emitted type (`for t in (..)_i64`).
            writeln!(
                out,
                "    // `for..until` break-generation capture \
                 (TASK-0341.02.01.05.02): -1 = did-not-converge sentinel."
            )
            .ok();
            writeln!(out, "    let mut __nuc_break_gen: i64 = -1;").ok();
            writeln!(out).ok();
            for e in &rewritten {
                crate::render_event(e, out, 1, ctx, None)?;
                if matches!(
                    e,
                    Event::Loop {
                        break_cond: Some(_),
                        ..
                    }
                ) {
                    emit_cap_hit_resolution(out, &info);
                }
            }
            Ok(())
        }
    }
}

/// Emit the `for..until` early-exit break check as the LAST statement of
/// the loop body — AFTER the body and AFTER any `check_frame` latency
/// measurement closes, so the final (break-causing) iteration is still
/// fully executed (and measured) before the loop terminates. The
/// predicate is a bool `IrExpr::Compare` over runtime data values (e.g.
/// `max_abs_diff < epsilon`), rendered via the scalar VALUE renderer
/// [`render_bool_expr`].
///
/// The CAPTURE (`__nuc_break_gen = {var}`) happens BEFORE the `break`, so
/// the recorded value is the last EXECUTED generation `var` (the body for
/// that iteration is already fully run). On cap-hit (the predicate never
/// fires) `__nuc_break_gen` stays at the -1 sentinel declared in
/// [`render_top_level_events`] — the cap-hit-not-converged observability
/// signal (TASK-0341.02.01.05.03), resolved post-loop by
/// [`emit_cap_hit_resolution`]. Caller emits this only when
/// `break_cond.is_some()`; a plain `for` loop emits nothing here
/// (byte-identical to the pre-S4 backend — the regression guard).
pub(crate) fn emit_break_check(
    out: &mut String,
    cond: &IrExpr,
    var: &str,
    body_pad: &str,
    body_ctx: &RenderCtx<'_>,
) -> Result<(), EmitError> {
    let cond_src = render_bool_expr(cond, body_ctx)?;
    writeln!(
        out,
        "{body_pad}if {cond_src} {{ __nuc_break_gen = {var}; break; }}"
    )
    .ok();
    Ok(())
}

/// What a `for..until` early-exit loop needs the surrounding `fn main`
/// to know so the runtime break-generation final-read
/// (TASK-0341.02.01.05.02) + cap-hit observability
/// (TASK-0341.02.01.05.03) can be wired. Collected by
/// [`collect_break_loop_info`] from a single-worker EventList.
///
/// - `cap` is the LAST valid generation index = `range.end - 1`. On a
///   `for t : 0 .. CAP+1` source loop this is `CAP`. It is both the
///   cap-hit final-read slice AND the value the cap-hit `__nuc_final_gen`
///   resolves to (the last generation the loop actually computed).
/// - `written` is the set of DataIds the loop body writes (indexed
///   outputs); ONLY reads of these arrays are eligible for the
///   final-read rewrite. A read of an unrelated array with a coincidental
///   `[CAP]` outer index must NOT be rewritten.
#[derive(Debug, Clone)]
pub(crate) struct BreakLoopInfo {
    /// LAST valid generation index (`range.end - 1`).
    pub(crate) cap: i64,
    /// DataIds the break loop body writes (final-read rewrite candidates).
    pub(crate) written: BTreeSet<DataId>,
}

/// Scan the TOP-LEVEL events for a `for..until` early-exit loop (an
/// `Event::Loop` carrying `break_cond: Some`). Returns the
/// [`BreakLoopInfo`] for the FIRST such loop, or `None` for every plain
/// program (byte-identical no-op — no `__nuc_break_gen` machinery is
/// emitted).
///
/// Single-worker scope (TASK-0341.02.01.06): the projection only ever
/// sets `break_cond` on an untagged SOURCE loop, and the single-worker
/// EventList carries at most one such loop in the examples this slice
/// ships (21-jacobi-converge). If a future program nests two `until`
/// loops the FIRST top-level one wins here; that is a documented
/// limitation (filed forward to S7) rather than a silent mis-handle —
/// multi-`until` is not yet expressible/required.
pub(crate) fn collect_break_loop_info(events: &[Event]) -> Option<BreakLoopInfo> {
    for e in events {
        if let Event::Loop {
            range,
            body,
            break_cond: Some(_),
            ..
        } = e
        {
            let mut written: BTreeSet<DataId> = BTreeSet::new();
            let mut _whole: BTreeSet<DataId> = BTreeSet::new();
            walk_fire_outputs(body, &mut _whole, &mut written);
            // A cap of `range.end - 1` is the last valid generation. An
            // empty range (end <= start) cannot host a break — guard
            // against a degenerate (end == i64::MIN) underflow with a
            // saturating sub; such a loop never executes a body so the
            // rewrite is inert anyway.
            return Some(BreakLoopInfo {
                cap: range.end.saturating_sub(1),
                written,
            });
        }
    }
    None
}

/// Rewrite the OUTER (generation-axis) index of every post-loop read of
/// a break-written array from the compile-time cap to the runtime
/// `__nuc_final_gen` (TASK-0341.02.01.05.02). STRUCTURAL — operates on
/// the `IrExpr` index, NOT on a rendered string (textual replace on a
/// rendered index is the `feedback-textual-replace-codegen-unsafe`
/// foot-gun). An index is rewritten iff: (a) the read is on an array in
/// `info.written`, AND (b) its outer index resolves to the constant
/// `info.cap` (a bare `IntLit(cap)` or an `Ident` whose `sidecar.consts`
/// value equals `cap`). Any other outer index (a loop var, a different
/// constant) is left untouched — the in-loop step reads
/// `field[(t+CAP)%(CAP+1)]` are an `Ident`/`BinOp`, never a bare `cap`
/// constant, so they are never rewritten (and they are inside the loop
/// where `t` is in scope, not in the post-loop extraction).
pub(crate) fn rewrite_final_reads(
    events: &mut [Event],
    info: &BreakLoopInfo,
    sidecar: &NameSidecar,
    in_break_loop: bool,
) {
    for e in events.iter_mut() {
        match e {
            Event::Fire { bindings, .. } => {
                if !in_break_loop {
                    for input in bindings.inputs.iter_mut() {
                        rewrite_arg_binding(input, info, sidecar);
                    }
                }
            }
            Event::Loop {
                body, break_cond, ..
            } => {
                // The reads INSIDE the break loop itself (the step / the
                // reduction) reference the live loop var, never a final-
                // read of the converged generation — skip them. Reads in
                // sibling/post loops (the extraction nest) are rewritten.
                let inside = in_break_loop || break_cond.is_some();
                rewrite_final_reads(body, info, sidecar, inside);
            }
            // Documented invariant, NOT an accidental silent skip
            // (cycle-262 architect P3-1): the final-read rewrite only
            // targets an INDEXED `Fire` input read of a break-written
            // array. In single-worker pthreads-sync scope a post-loop
            // `Push`/`Wait`/`Sync` cannot occur (no inter-worker transfer
            // — multi-worker `break_cond` is fail-loud rejected in the
            // multi_worker_walker / tcp_plan walkers), and `Alloc`/`Free`
            // carry no readable index expression. So no other variant can
            // host a rewritable read here; if that ever changes (a
            // multi-worker break in S7), this arm must grow a case rather
            // than silently pass (feedback-option-none-skip-arm-silent-drop).
            _ => {}
        }
    }
}

/// Rewrite one [`ArgBinding`] in place (recurses into nested calls).
fn rewrite_arg_binding(b: &mut ArgBinding, info: &BreakLoopInfo, sidecar: &NameSidecar) {
    match b {
        ArgBinding::Data(slice) => rewrite_data_slice(slice, info, sidecar),
        ArgBinding::Nested { args, .. } => {
            for a in args.iter_mut() {
                rewrite_arg_binding(a, info, sidecar);
            }
        }
        ArgBinding::Scalar(_) => {}
    }
}

/// Rewrite the outer index of one read [`DataSlice`] if it is a cap-equal
/// constant read of a break-written array. See [`rewrite_final_reads`].
fn rewrite_data_slice(slice: &mut DataSlice, info: &BreakLoopInfo, sidecar: &NameSidecar) {
    if !info.written.contains(&slice.data) {
        return;
    }
    let Some(outer) = slice.indices.first_mut() else {
        return;
    };
    if outer_index_equals_cap(outer, info.cap, sidecar) {
        // `__nuc_final_gen` is an `i64` local in scope at the extraction;
        // render_int_expr emits a bare `Ident` as-is (not a const, not in
        // abs_subst), so `field[(__nuc_final_gen) * STRIDE + ...]` results.
        *outer = IrExpr::Ident("__nuc_final_gen".to_string());
    }
}

/// True iff the outer-index expression is a compile-time constant equal
/// to `cap`: a bare `IntLit(cap)`, or an `Ident` whose `sidecar.consts`
/// value is `cap`. Anything else (a loop var, an arithmetic expression,
/// a different constant) is false.
fn outer_index_equals_cap(outer: &IrExpr, cap: i64, sidecar: &NameSidecar) -> bool {
    match outer {
        IrExpr::IntLit(v) => *v == cap,
        IrExpr::Ident(n) => sidecar.consts.get(n).map(|c| c.value) == Some(cap),
        _ => false,
    }
}

/// Emit the post-loop cap-hit-not-converged observability block + the
/// `__nuc_final_gen` resolution (TASK-0341.02.01.05.03 / .05.02). Placed
/// immediately after the break loop, before the extraction reads.
///
/// CHOSEN SEMANTICS (.05.03 AC#2): a runtime STDERR diagnostic. Cap-hit
/// (the loop ran the full cap without the predicate ever firing) is
/// distinguished from a converged early-exit by the `-1` sentinel and is
/// announced on stderr — NOT a silent stop-at-N that looks byte-identical
/// to convergence (the `feedback-option-none-skip-arm-silent-drop`
/// anti-pattern). Stderr (not stdout / output.bin) keeps the
/// cross-backend differential's success-path bytes untouched: the
/// observability signal is determinism-safe.
///
/// `__nuc_final_gen` = `__nuc_break_gen` on convergence (the captured
/// runtime k), = `CAP` on cap-hit (so the extraction reads the LAST
/// computed generation `field[CAP]`, which IS materialised). This makes
/// a cap-hit run extract a valid (last) generation rather than the
/// unwritten sentinel slice, while STILL being observable via the stderr
/// line.
pub(crate) fn emit_cap_hit_resolution(out: &mut String, info: &BreakLoopInfo) {
    let cap = info.cap;
    writeln!(out).ok();
    writeln!(
        out,
        "    // Cap-hit-not-converged observability \
         (TASK-0341.02.01.05.03): stderr diagnostic, NOT a silent stop-at-cap."
    )
    .ok();
    writeln!(out, "    if __nuc_break_gen < 0 {{").ok();
    writeln!(
        out,
        "        eprintln!(\"[[nuc_converge]] did NOT converge within the cap \
         ({cap} + 1 generations); extracting the last computed generation {cap}\");"
    )
    .ok();
    writeln!(out, "    }}").ok();
    // The runtime generation the extraction reads: the captured break
    // generation, or the cap (last computed) on cap-hit.
    writeln!(
        out,
        "    let __nuc_final_gen: i64 = if __nuc_break_gen < 0 {{ {cap}_i64 }} \
         else {{ __nuc_break_gen }};"
    )
    .ok();
    writeln!(out).ok();
}
