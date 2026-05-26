//! The shared per-worker event walker (TASK-0239 / TASK-0181 /
//! TASK-0270). `render_worker_events` walks one worker's projected
//! EventList and emits Rust statements; consumed by pthreads-sync,
//! pthreads-async, and mp-tcp-event (mp-tcp-bufsync bypasses this
//! walker entirely — see the parent [`super`] module doc).

use std::fmt::Write as _;

use nucleus_compiler::event::{Event, IterVar, ViolationKind, WorkerId};

use crate::check_frame::{emit_count_branch, emit_log_branch, sanitize_loop_var};
use crate::render::{
    render_const_expr_pub, render_fire_args_pub, render_fire_output_assign_pub,
    render_reuse_buf_decls_pub, render_reuse_marker_comment, render_reuse_per_iter_update_pub,
    EmitError, RenderCtxPub,
};

use super::block_tag::compute_block_tag_abs_exprs;
use super::ctx::WalkerCtx;
use super::wait::render_wait_assign;

/// Walk one worker's EventList, emitting Rust statements into `out`.
///
/// This is the SHARED walker — pthreads-sync's `Plan`, pthreads-
/// async's `Plan`, and mp-tcp-event's `Plan` all call through it
/// (`rendezvous_prefix` = `"slot"` / `"ring"` / `"chan"` respectively;
/// mp-tcp-bufsync is the fourth tier-1 backend but bypasses this
/// walker and calls `render_wait_assign` directly). The substitution
/// surface is exactly `ctx.rendezvous_prefix` (the variable-name
/// prefix on `{prefix}_<id>.push(...)` / `{prefix}_<id>.wait()`).
///
/// # Strip-mine rebinding (TASK-0181)
///
/// A `block_tag.is_some()` `Event::Loop` is per-occurrence absolute-
/// index rebound exactly as the single-worker path does (TASK-0180).
/// The rebinding map is threaded through `RenderCtxPub.abs_subst` so
/// it reaches every Fire arg / index / const-expr render site, not
/// just the loop header (the subtle TASK-0181 review-gate finding:
/// substituting only at the header would leave Fire body uses un-
/// rebound, re-introducing the accumulator double-count TASK-0180
/// closed). No tier-1 multi-worker schedule blocks today, so this
/// path is structurally unreachable from the e2e matrix; the rebinding
/// is exercised by unit tests in `backend-common/tests/`.
///
/// # Per-worker partition override (TASK-0212)
///
/// When the partition pass recorded a per-worker slice for THIS
/// worker on this iter var (`sidecar.partition_worker_ranges`), the
/// loop renders the concrete literal range. Otherwise the source-form
/// symbolic / literal precedence from `sidecar.loop_bounds` applies.
/// The strip-mine path above renders the concrete folded range
/// instead and skips the partition-slice check (the strip-mined inner
/// loop iterates `0..N`, not the partitioned source slice).
///
/// # check_frame defense
///
/// `check_frame.is_some() && block_tag.is_some()` is an
/// `inject_check_frames`-layer invariant violation (frames attach
/// only to outer source loops). The strip-mine path above returns
/// early before reaching this branch, so the defense is structurally
/// unreachable today, but catches a future projection-layer regression.
pub fn render_worker_events(
    ctx: &WalkerCtx<'_>,
    worker: WorkerId,
    events: &[Event],
    out: &mut String,
    indent: usize,
    prefix: &str,
) -> Result<(), EmitError> {
    let render_ctx = ctx.render_ctx();
    render_worker_events_inner(ctx, worker, events, out, indent, prefix, &render_ctx, None)
}

/// `enclosing` is the iter-var of the immediately-enclosing
/// `Event::Loop` (the tile loop, when the child is a strip-mined
/// inner-block loop with `block_tag.is_partial == false`). `None` at
/// top level. Mirrors the single-worker `render_events_in` /
/// `render_event` parameter (TASK-0180).
///
/// Eight params is one over clippy's `too_many_arguments` threshold;
/// the alternative (a single bundle struct) would be a synthetic
/// container with no semantic content — the parameters are the
/// genuine inputs to one stateless event-walk step. Local allow.
#[allow(clippy::too_many_arguments)]
fn render_worker_events_inner(
    ctx: &WalkerCtx<'_>,
    worker: WorkerId,
    events: &[Event],
    out: &mut String,
    indent: usize,
    prefix: &str,
    render_ctx: &RenderCtxPub<'_>,
    enclosing: Option<IterVar>,
) -> Result<(), EmitError> {
    let pad = "    ".repeat(indent);
    let rendezvous_prefix = ctx.rendezvous_prefix;
    for e in events {
        match e {
            Event::Fire {
                kernel, bindings, ..
            } => {
                let callee = ctx.names.kernel.get(kernel).ok_or_else(|| {
                    EmitError::ContractGap(format!(
                        "kernel id {kernel:?} in a Fire has no name in NameTables"
                    ))
                })?;
                let args = render_fire_args_pub(*kernel, &bindings.inputs, render_ctx)?;
                match &bindings.output {
                    None => {
                        writeln!(out, "{pad}kernels::{callee}({args});").ok();
                    }
                    Some(o) if o.indices.is_empty() => {
                        let name = ctx.data_name(o.data)?;
                        writeln!(out, "{pad}let mut {name} = kernels::{callee}({args});").ok();
                    }
                    Some(o) => {
                        // TASK-0209 shared scalar-vs-sub-array
                        // classifier — all `render_worker_events`-
                        // using backends (pthreads-sync, pthreads-
                        // async, mp-tcp-event) route Fire-output
                        // assignment through `render_fire_output_
                        // assign_pub` so the Fire-output sites
                        // cannot drift across backends.
                        let rhs = format!("kernels::{callee}({args})");
                        let stmt = render_fire_output_assign_pub(o, &rhs, render_ctx)?;
                        writeln!(out, "{pad}{stmt}").ok();
                    }
                }
            }
            Event::Loop {
                iter_var,
                range,
                body,
                block_tag,
                check_frame,
            } => {
                let var = ctx.names.iter_var.get(iter_var).ok_or_else(|| {
                    EmitError::ContractGap(format!(
                        "iter var {iter_var:?} in Event::Loop has no name in NameTables"
                    ))
                })?;

                // Per-occurrence absolute-index rebinding (TASK-0181;
                // mirrors TASK-0180 on the single-worker path) AND
                // reuse circular-buffer codegen (TASK-0270, mirrors
                // TASK-0269 strip-mine arm on the single-worker path).
                //
                // Pre-TASK-0270 this arm delegated wholesale to
                // [`render_block_tag_loop_header`] (which writes the
                // for-header and returns the rebound child). With the
                // reuse-buffer landing the order matters: the buffer
                // decl + initial-fill prologue MUST live OUTSIDE the
                // for-header so the buffer persists across iterations.
                // We therefore split the helper into the pure
                // [`compute_block_tag_abs_exprs`] (returns abs +
                // strip_lo_expr) + an inline header write, so the
                // walker can interpose `render_reuse_buf_decls_pub`
                // between them.
                //
                // `strip_lo_expr` is the `iv=0` counterpart of `abs`
                // built STRUCTURALLY (NOT via textual
                // `abs.replace(var, "0_i64")` — that defect is filed
                // in memory `feedback-textual-replace-codegen-unsafe`,
                // cycle 103 architect P1.1 on the single-worker path,
                // mirrored here).
                if let Some(tag) = block_tag {
                    let (abs, strip_lo_expr) =
                        compute_block_tag_abs_exprs(*iter_var, tag, enclosing, render_ctx)?;
                    // TASK-0270 strip-mine arm reuse-buffer decl +
                    // prologue. Emitted at the OUTER pad above the
                    // for-header so the buffer persists across the
                    // inner loop's iterations. The prologue's
                    // reuse-axis "lo" uses the rebound ABSOLUTE
                    // expression at iv=0 (`LO + tile*N + 0`) because
                    // the strip-mined loop's lexical range is
                    // `0..inner_len`, not `LO..HI`. NO-OP when the iv
                    // carries no reuse (every shipped non-blocked
                    // schedule).
                    let reuse_groups = render_reuse_buf_decls_pub(
                        out,
                        indent,
                        *iter_var,
                        var,
                        &strip_lo_expr,
                        body,
                        render_ctx,
                    )?;
                    // Build the child context carrying BOTH the
                    // strip-mine abs rebinding AND the reuse-active
                    // groups. Parent's reuse_active is preserved (so
                    // nested reuse loops compose); new groups OVERRIDE
                    // on data_id collision.
                    let mut child_subst = render_ctx.abs_subst.clone();
                    child_subst.insert(var.clone(), abs.clone());
                    let mut child_reuse = render_ctx.reuse_active.clone();
                    for (data_id, gs) in reuse_groups.clone() {
                        child_reuse.insert(data_id, gs);
                    }
                    let child = render_ctx
                        .with_abs_subst_and_reuse_active(child_subst, child_reuse);
                    // Header line: concrete folded range
                    // (`{start}_i64..{end}_i64`) — NOT the source-form
                    // bound (would re-introduce the full range) and
                    // NOT the partition slice (the strip-mined inner
                    // loop iterates over the tile, not the worker's
                    // partition slice). Inlined here because the buf
                    // decls above had to land BEFORE the for-header;
                    // the regular `render_block_tag_loop_header`
                    // wrapper is unchanged and continues to be used by
                    // mp-tcp-bufsync's strip-mine arm (which does not
                    // emit reuse codegen on this cycle).
                    writeln!(
                        out,
                        "{pad}for {var} in ({}_i64)..({}_i64) {{",
                        range.start, range.end
                    )
                    .ok();
                    // TASK-0265 Tier 1 marker (preserved): a
                    // strip-mined inner loop CAN carry reuse (cf.
                    // 05-stencil/distributed `loop x : block=64,
                    // vectorize=8, reuse;`). Emit the marker at body
                    // entry of the inner-block loop, same as the
                    // non-strip-mine path below. The marker substring
                    // `reuse_widths_pending` is preserved as a
                    // regression canary above the buffer decls (which
                    // landed above the for-header) — the
                    // `__reuse_buf_<data>_a<axis>_g<group_idx>` +
                    // `rem_euclid(L_i64)` strings are the second-layer
                    // codegen canary (TASK-0282: `_g<group_idx>` is
                    // uniform; single-group cases carry `_g0`).
                    render_reuse_marker_comment(
                        out,
                        indent + 1,
                        *iter_var,
                        var,
                        ctx.sidecar,
                        ctx.names,
                    );
                    // TASK-0270 per-iter update: load the most-distant
                    // element into the buffer slot before any Fire arg
                    // reads it. Iv expression here is the rebound
                    // ABSOLUTE expression (so the source-array index
                    // reflects the strip-mined coordinate), NOT the
                    // bare `var`.
                    render_reuse_per_iter_update_pub(
                        out,
                        indent + 1,
                        &reuse_groups,
                        &abs,
                        &child,
                    )?;
                    render_worker_events_inner(
                        ctx,
                        worker,
                        body,
                        out,
                        indent + 1,
                        prefix,
                        &child,
                        Some(*iter_var),
                    )?;
                    writeln!(out, "{pad}}}").ok();
                    continue;
                }

                // Defense-in-depth invariant
                // (`inject_check_frames` is contracted to populate
                // check_frame only on outer source loops; block_tag ==
                // None). The strip-mine path above returns early, so
                // reaching here with both set is a projection-layer
                // bug.
                if check_frame.is_some() && block_tag.is_some() {
                    return Err(EmitError::ContractGap(format!(
                        "Event::Loop for iter var `{var}` carries BOTH a check_frame \
                         and a block_tag — `inject_check_frames` is contracted to \
                         populate check_frame only on outer source loops; this is a \
                         projection-layer bug (TASK-0052.05 multi-worker invariant)."
                    )));
                }
                // Per-worker partition override (TASK-0212): if the
                // partition pass recorded a slice for THIS worker on
                // this iter var, render the concrete literal range.
                // The symbolic `loop_bounds` entry names the SOURCE
                // range, not the partitioned slice. A worker not
                // listed in the per-iter-var map (e.g. host, which
                // doesn't participate in partition=workers) falls
                // through to the source-form symbolic / literal
                // precedence exactly as before TASK-0212.
                let partition_slice = ctx
                    .sidecar
                    .partition_worker_ranges
                    .get(iter_var)
                    .and_then(|m| m.get(&worker));
                let (lo, hi) = match partition_slice {
                    Some(r) => (format!("{}_i64", r.start), format!("{}_i64", r.end)),
                    None => match ctx.sidecar.loop_bounds.get(iter_var) {
                        Some(b) => (
                            render_const_expr_pub(&b.lo, render_ctx)?,
                            render_const_expr_pub(&b.hi, render_ctx)?,
                        ),
                        None => (format!("{}_i64", range.start), format!("{}_i64", range.end)),
                    },
                };
                // TASK-0270 regular (non-strip-mine) arm: emit reuse-
                // buffer decl + initial-fill prologue at OUTER pad
                // BEFORE the for-header (the buffer must persist
                // across iterations). The prologue's reuse-axis "lo"
                // is the PER-WORKER lo computed above — when
                // `partition_worker_ranges` recorded a slice, the
                // prologue uses that worker's first-iteration absolute
                // coordinate, which is the correct source-array index
                // for the prologue fill. NO-OP when the iv carries no
                // reuse (every shipped multi-worker schedule pre-
                // TASK-0270).
                let reuse_groups = render_reuse_buf_decls_pub(
                    out,
                    indent,
                    *iter_var,
                    var,
                    &lo,
                    body,
                    render_ctx,
                )?;
                writeln!(out, "{pad}for {var} in ({lo})..({hi}) {{").ok();
                let body_indent = indent + 1;
                let body_pad = "    ".repeat(body_indent);
                // TASK-0265 Tier 1 marker (preserved): regular (non-
                // strip-mined) loop — marker comment at body entry.
                // The substring `reuse_widths_pending` is the first-
                // layer canary (grep-able for AC#4 of TASK-0265). The
                // `__reuse_buf_<data>_a<axis>_g<group_idx>` +
                // `rem_euclid(L_i64)` strings the buf_decls above emit
                // are the second-layer codegen canary (TASK-0282:
                // `_g<group_idx>` is uniform; single-group cases carry
                // `_g0`). NO-OP when the iv carries no reuse.
                render_reuse_marker_comment(
                    out,
                    body_indent,
                    *iter_var,
                    var,
                    ctx.sidecar,
                    ctx.names,
                );
                // Build the body's child RenderCtxPub carrying the
                // newly-active reuse groups. Parent's abs_subst is
                // preserved (non-strip-mine arm does NOT introduce a
                // rebinding — only the strip-mine arm above does);
                // new reuse groups OVERRIDE on data_id collision (a
                // hypothetical inner-loop reuse on the SAME data; not
                // exercised by 05-stencil/distributed but BTreeMap
                // semantics are well-defined). Per-iter-update +
                // recursion both consume this child.
                let mut child_reuse = render_ctx.reuse_active.clone();
                for (data_id, gs) in reuse_groups.clone() {
                    child_reuse.insert(data_id, gs);
                }
                let body_ctx = render_ctx.with_reuse_active(child_reuse);
                // TASK-0270 per-iter update: load the most-distant
                // element into the buffer slot before any Fire arg
                // reads it. Iv expression here is the bare var (no
                // abs_subst rebinding on this non-strip-mine path).
                render_reuse_per_iter_update_pub(
                    out,
                    body_indent,
                    &reuse_groups,
                    var,
                    &body_ctx,
                )?;
                if let Some(frame) = check_frame {
                    // TASK-0221 defensive — `var` (NameTables) and
                    // `frame.loop_var` (CheckFrame) must name the same
                    // user-source loop variable. Dev-only assert
                    // catches future projection divergence.
                    debug_assert_eq!(
                        var.as_str(),
                        frame.loop_var.as_str(),
                        "CheckFrame.loop_var diverged from NameTables.iter_var \
                         (projection-layer bug; TASK-0221)"
                    );
                    writeln!(
                        out,
                        "{body_pad}let _check_start = std::time::Instant::now();"
                    )
                    .ok();
                    render_worker_events_inner(
                        ctx,
                        worker,
                        body,
                        out,
                        body_indent,
                        prefix,
                        &body_ctx,
                        Some(*iter_var),
                    )?;
                    writeln!(
                        out,
                        "{body_pad}let _check_elapsed = _check_start.elapsed().as_nanos();"
                    )
                    .ok();
                    match frame.on_violation {
                        ViolationKind::Panic => {
                            writeln!(
                                out,
                                "{body_pad}if _check_elapsed > {ns}_u128 {{ panic!(\"latency budget violated on `check loop {lv}`: iteration took {{}} ns, max {ns} ns\", _check_elapsed); }}",
                                ns = frame.latency_max_ns,
                                lv = frame.loop_var,
                            )
                            .ok();
                        }
                        ViolationKind::Log => {
                            // TASK-0222: shared template — see emit_log_branch.
                            emit_log_branch(out, &body_pad, &frame.loop_var, frame.latency_max_ns);
                        }
                        ViolationKind::Count => {
                            // TASK-0222: shared template — see emit_count_branch.
                            let id = sanitize_loop_var(&frame.loop_var);
                            emit_count_branch(out, &body_pad, &id, frame.latency_max_ns);
                        }
                    }
                } else {
                    render_worker_events_inner(
                        ctx,
                        worker,
                        body,
                        out,
                        indent + 1,
                        prefix,
                        &body_ctx,
                        Some(*iter_var),
                    )?;
                }
                writeln!(out, "{pad}}}").ok();
            }
            Event::Sync { sync, .. } => {
                // Barrier identity is the contract-carried SyncTag
                // (TASK-0172): every participant of this barrier
                // carries the same tag, so all participants .wait()
                // on the same `bar_<tag>` with no pre-order-index
                // recovery.
                let bid = sync.0;
                writeln!(out, "{pad}{prefix}bar_{bid}.wait();").ok();
            }
            Event::Push { data, dst, seq, .. } => {
                let rid = ctx.rendezvous_ids.get(&(*data, *seq)).ok_or_else(|| {
                    EmitError::ContractGap(format!(
                        "Push of data {data:?} (seq {seq:?}) has no rendezvous id \
                         (not collected as cross-worker)"
                    ))
                })?;
                let name = ctx.data_name(*data)?;
                let to = ctx.worker_name(*dst);
                writeln!(
                    out,
                    "{pad}{prefix}{rendezvous_prefix}_{rid}.push({name}.clone()); // send `{name}` to {to}",
                )
                .ok();
            }
            Event::Wait { data, src, seq, .. } => {
                let rid = ctx.rendezvous_ids.get(&(*data, *seq)).ok_or_else(|| {
                    EmitError::ContractGap(format!(
                        "Wait of data {data:?} (seq {seq:?}) has no rendezvous id \
                         (not collected as cross-worker)"
                    ))
                })?;
                let name = ctx.data_name(*data)?;
                let from = ctx.worker_name(*src);
                // TASK-0117 host-side gather: see render_wait_assign.
                let assign = render_wait_assign(
                    ctx.sidecar,
                    ctx.pair_tiles,
                    &name,
                    *data,
                    *seq,
                    &format!("{prefix}{rendezvous_prefix}_{rid}.wait()"),
                )?;
                writeln!(out, "{pad}{assign} // recv `{name}` from {from}",).ok();
            }
            Event::Alloc { .. } | Event::Free { .. } => {
                // RAII Vec storage; no explicit reservation.
            }
        }
    }
    Ok(())
}
