//! `Plan::render_events` — the per-worker event-walk codegen.
//! Originally inline in `lib.rs` before the slice-4 split.
//!
//! Walks the projection's `Event` list emitting Rust for each Fire /
//! Loop / Sync / Push / Wait; recurses into Loop bodies with a child
//! `RenderCtxPub`. Per-occurrence absolute-index rebinding for
//! strip-mined block-tag loops, partition-slice override, reuse-codegen
//! buffer prologue, check-frame instrumentation, and host-mediated
//! barrier emission are all dispatched here.

use std::fmt::Write as _;

use backend_common::multi_worker_walker::render_wait_assign;
use backend_common::render::{
    render_const_expr_pub, render_fire_args_pub, render_reuse_buf_decls_pub,
    render_reuse_marker_comment, render_reuse_per_iter_update_pub, RenderCtxPub,
};
use nucleus_compiler::event::{Event, IterVar, WorkerId};

use crate::encode::{decode_expr, encode_expr};
use crate::EmitError;

use super::Plan;

impl Plan<'_> {
    /// `enclosing` is the iter-var of the immediately-enclosing
    /// `Event::Loop` (the tile loop, when the child is a strip-mined
    /// inner-block loop with `block_tag.is_partial == false`). `None`
    /// at top level. Mirrors the pthreads-sync single-worker
    /// `render_events_in` parameter (TASK-0180 / TASK-0181).
    ///
    /// Eight params is one over clippy's `too_many_arguments`
    /// threshold; bundling them into a struct would be synthetic
    /// container ceremony for what is a stateless event-walk step
    /// with genuine per-call inputs. Local allow (same rationale as
    /// the shared `multi_worker_walker::render_worker_events_inner`).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_events(
        &self,
        events: &[Event],
        out: &mut String,
        indent: usize,
        worker: WorkerId,
        is_host: bool,
        ctx: &RenderCtxPub<'_>,
        enclosing: Option<IterVar>,
    ) -> Result<(), EmitError> {
        let pad = "    ".repeat(indent);
        for e in events {
            match e {
                Event::Fire {
                    kernel, bindings, ..
                } => {
                    let callee = self.names.kernel.get(kernel).ok_or_else(|| {
                        EmitError::ContractGap(format!(
                            "kernel id {kernel:?} in a Fire has no name in NameTables"
                        ))
                    })?;
                    // SHARED renderer — no drift from pthreads-sync.
                    let args = render_fire_args_pub(*kernel, &bindings.inputs, ctx)?;
                    match &bindings.output {
                        None => {
                            writeln!(out, "{pad}kernels::{callee}({args});").ok();
                        }
                        Some(o) if o.indices.is_empty() => {
                            let name = self.data_name(o.data)?;
                            writeln!(out, "{pad}let mut {name} = kernels::{callee}({args});").ok();
                        }
                        Some(o) => {
                            // TASK-0209: shared scalar-vs-sub-array
                            // classifier via the pthreads-sync helper.
                            // Same impl as the pthreads-sync single-
                            // and multi-worker Fire-output sites — no
                            // codegen drift between backends, which
                            // the cross-backend bit-identical
                            // differential (PRD §10.1) depends on.
                            let rhs = format!("kernels::{callee}({args})");
                            let stmt = backend_common::render::render_fire_output_assign_pub(
                                o, &rhs, ctx,
                            )?;
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
                    let var = self.names.iter_var.get(iter_var).ok_or_else(|| {
                        EmitError::ContractGap(format!(
                            "iter var {iter_var:?} in Event::Loop has no name in NameTables"
                        ))
                    })?;

                    // Per-occurrence absolute-index rebinding (TASK-0181;
                    // mirrors pthreads-sync single-worker TASK-0180). Now
                    // delegates to the SHARED
                    // `backend_common::multi_worker_walker::
                    // render_block_tag_loop_header` (TASK-0253) — the same
                    // helper the pthreads-sync / pthreads-async multi-worker
                    // walker calls. The strip-mined inner loop HEADER and
                    // the rebound child `RenderCtxPub` (with `abs_subst`
                    // extended so every Fire arg / index / inner-bound site
                    // substitutes — NOT just the header; the load-bearing
                    // TASK-0181 review-gate finding) are emitted by the
                    // helper. This arm owns only the body recursion through
                    // mp-tcp-bufsync's per-backend substrate (TCP
                    // `ctrl_<peer>` / `sock_<peer>` barriers + host-vs-
                    // worker dispatch in `render_worker_program`) and the
                    // closing `}`. The previous arrangement (TASK-0181
                    // cycle 73) duplicated the rebinding arithmetic
                    // across two files; with this delegation, the
                    // arithmetic lives in exactly one place across all
                    // MULTI-worker backends (cycle-75 review hardening:
                    // a separate sibling copy persists on the
                    // pthreads-sync SINGLE-worker render path, which
                    // uses backend-private RenderCtx — see the helper's
                    // doc-comment for the RenderCtx <-> RenderCtxPub
                    // unification note).
                    if let Some(tag) = block_tag {
                        // TASK-0284 cycle 107: parity with the shared
                        // `multi_worker_walker` strip-mine arm reuse
                        // codegen (TASK-0270 cycle 104). Buffer decls +
                        // prologue MUST live OUTSIDE the for-header (the
                        // buffer must persist across the inner loop's
                        // iterations), so the previous wholesale
                        // delegation to `render_block_tag_loop_header`
                        // (which writes the header itself) is split: use
                        // `compute_block_tag_abs_exprs` for the pure
                        // expressions (returns abs + structurally-built
                        // strip_lo_expr — NO textual replace, mirrors
                        // the cycle-103 fix in pthreads-sync per
                        // `feedback-textual-replace-codegen-unsafe`),
                        // emit buf decls at the OUTER pad, write the
                        // for-header inline, then emit per-iter update
                        // + recurse into body with the child context
                        // carrying BOTH abs_subst AND reuse_active.
                        // `render_block_tag_loop_header` is still
                        // used by callers that don't want reuse codegen
                        // (none currently — pthreads-async / mp-tcp-event
                        // moved to the same pattern in cycle 104).
                        let var_string = var.clone();
                        let (abs, strip_lo_expr) =
                            backend_common::multi_worker_walker::compute_block_tag_abs_exprs(
                                *iter_var, tag, enclosing, ctx,
                            )?;
                        let reuse_groups = render_reuse_buf_decls_pub(
                            out,
                            indent,
                            *iter_var,
                            &var_string,
                            &strip_lo_expr,
                            body,
                            ctx,
                        )?;
                        let mut child_subst = ctx.abs_subst.clone();
                        child_subst.insert(var_string.clone(), abs.clone());
                        let mut child_reuse = ctx.reuse_active.clone();
                        for (data_id, gs) in reuse_groups.clone() {
                            child_reuse.insert(data_id, gs);
                        }
                        let child_ctx =
                            ctx.with_abs_subst_and_reuse_active(child_subst, child_reuse);
                        // Header line: concrete folded range
                        // (`{start}_i64..{end}_i64`) — NOT the source-form
                        // bound (would re-introduce the full range) and
                        // NOT the partition slice (the strip-mined inner
                        // loop iterates over the tile, not the worker's
                        // partition slice).
                        writeln!(
                            out,
                            "{pad}for {var_string} in ({}_i64)..({}_i64) {{",
                            range.start, range.end
                        )
                        .ok();
                        // Marker (preserved; substring `reuse_widths_pending`
                        // is load-bearing for the cross-backend grep tests).
                        render_reuse_marker_comment(
                            out,
                            indent + 1,
                            *iter_var,
                            &var_string,
                            ctx.sidecar,
                            ctx.names,
                        );
                        // Per-iter update: iv expression is the rebound
                        // ABSOLUTE expression so the source-array index
                        // reflects the strip-mined coordinate.
                        render_reuse_per_iter_update_pub(
                            out,
                            indent + 1,
                            &reuse_groups,
                            &abs,
                            &child_ctx,
                        )?;
                        self.render_events(
                            body,
                            out,
                            indent + 1,
                            worker,
                            is_host,
                            &child_ctx,
                            Some(*iter_var),
                        )?;
                        writeln!(out, "{pad}}}").ok();
                        continue;
                    }
                    // Per-worker partition override (TASK-0212): if the
                    // partition pass recorded a slice for THIS worker on
                    // this iter var, render the concrete literal range.
                    // See pthreads-sync multi_worker for the precedence
                    // rationale (concrete-per-worker > symbolic-source-
                    // form > concrete-folded fallback).
                    let partition_slice = self
                        .sidecar
                        .partition_worker_ranges
                        .get(iter_var)
                        .and_then(|m| m.get(&worker));
                    let (lo, hi) = match partition_slice {
                        Some(r) => (format!("{}_i64", r.start), format!("{}_i64", r.end)),
                        None => match self.sidecar.loop_bounds.get(iter_var) {
                            Some(b) => (
                                render_const_expr_pub(&b.lo, ctx)?,
                                render_const_expr_pub(&b.hi, ctx)?,
                            ),
                            None => (format!("{}_i64", range.start), format!("{}_i64", range.end)),
                        },
                    };
                    // TASK-0284 cycle 107: regular arm reuse codegen
                    // parity with the shared walker (TASK-0270 cycle
                    // 104). Buffer decls + prologue at OUTER pad BEFORE
                    // the for-header; per-iter update + body recursion
                    // inside the loop with a child context carrying the
                    // reuse_active map. NO-OP when the iv carries no
                    // reuse (preserves byte-identicality on every
                    // mp-tcp-bufsync cell shipped pre-TASK-0284).
                    let reuse_groups = render_reuse_buf_decls_pub(
                        out, indent, *iter_var, var, &lo, body, ctx,
                    )?;
                    writeln!(out, "{pad}for {var} in ({lo})..({hi}) {{").ok();
                    // Real-time `check loop V : latency_max=T` codegen
                    // (TASK-0052.02). Mirrors the pthreads-sync
                    // single-worker emit: `Instant::now()` at iter
                    // start, comparison + panic at iter end. Determinism
                    // preserved: the emitted bytes on the success path
                    // are unchanged (the instant is consumed locally,
                    // never written to wire / stdout), and panic exits
                    // with rustc's standard code 101 — the cross-backend
                    // differential treats "exit 101 + empty stdout" as
                    // an assertion signal, NOT a corrupt-output false
                    // positive.
                    //
                    // Test coverage: the emit-string pattern is pinned
                    // by `mp_tcp_bufsync_emit_includes_panic_instrumentation_on_check_loop`
                    // (TASK-0052.02 review-gate finding #2). No tier-1
                    // e2e cell uses `check loop` today; the
                    // string-assertion test is the lower-bound
                    // verification that this backend emits the
                    // contracted shape.
                    let body_indent = indent + 1;
                    let body_pad = "    ".repeat(body_indent);
                    // TASK-0284 cycle 107: marker + per-iter update at
                    // body entry (mirrors the shared walker regular
                    // arm). Marker substring `reuse_widths_pending`
                    // preserved as cross-backend canary. Both the
                    // check_frame and non-check_frame body-recursion
                    // arms below use `body_ctx` (the child ctx carrying
                    // the new `reuse_active` map) so any DataRef
                    // rewrite reaches the body.
                    render_reuse_marker_comment(
                        out,
                        body_indent,
                        *iter_var,
                        var,
                        ctx.sidecar,
                        ctx.names,
                    );
                    let mut child_reuse = ctx.reuse_active.clone();
                    for (data_id, gs) in reuse_groups.clone() {
                        child_reuse.insert(data_id, gs);
                    }
                    let body_ctx = ctx.with_reuse_active(child_reuse);
                    render_reuse_per_iter_update_pub(
                        out,
                        body_indent,
                        &reuse_groups,
                        var,
                        &body_ctx,
                    )?;
                    if let Some(frame) = check_frame {
                        // TASK-0221 (a): defensive — `var` (NameTables)
                        // and `frame.loop_var` (CheckFrame) must name
                        // the same user-source loop variable. Dev-only
                        // assert catches future projection divergence.
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
                        self.render_events(
                            body,
                            out,
                            body_indent,
                            worker,
                            is_host,
                            &body_ctx,
                            Some(*iter_var),
                        )?;
                        writeln!(
                            out,
                            "{body_pad}let _check_elapsed = _check_start.elapsed().as_nanos();"
                        )
                        .ok();
                        match frame.on_violation {
                            nucleus_compiler::event::ViolationKind::Panic => {
                                writeln!(
                                    out,
                                    "{body_pad}if _check_elapsed > {ns}_u128 {{ panic!(\"latency budget violated on `check loop {lv}`: iteration took {{}} ns, max {ns} ns\", _check_elapsed); }}",
                                    ns = frame.latency_max_ns,
                                    lv = frame.loop_var,
                                )
                                .ok();
                            }
                            nucleus_compiler::event::ViolationKind::Log => {
                                // TASK-0052.04. eprintln per violation;
                                // execution continues. Mirrors the
                                // pthreads-sync emit verbatim — the
                                // cross-backend differential test pins
                                // this in
                                // `mp_tcp_bufsync_emit_includes_log_eprintln_on_check_loop`.
                                // TASK-0222: shared template — see emit_log_branch.
                                backend_common::check_frame::emit_log_branch(
                                    out,
                                    &body_pad,
                                    &frame.loop_var,
                                    frame.latency_max_ns,
                                );
                            }
                            nucleus_compiler::event::ViolationKind::Count => {
                                // TASK-0052.04. The static counter +
                                // Drop guard are emitted at file scope
                                // by `render_worker_program` above
                                // (`collect_count_check_frames` walks
                                // the SAME events). Relaxed ordering is
                                // sufficient: the fetch_add and the
                                // Drop-time load both happen on the
                                // worker process's main thread.
                                // TASK-0222: shared template — see emit_count_branch.
                                let id =
                                    backend_common::check_frame::sanitize_loop_var(&frame.loop_var);
                                backend_common::check_frame::emit_count_branch(
                                    out,
                                    &body_pad,
                                    &id,
                                    frame.latency_max_ns,
                                );
                            }
                        }
                    } else {
                        self.render_events(
                            body,
                            out,
                            body_indent,
                            worker,
                            is_host,
                            &body_ctx,
                            Some(*iter_var),
                        )?;
                    }
                    writeln!(out, "{pad}}}").ok();
                }
                Event::Sync {
                    participants, sync, ..
                } => {
                    // Barrier identity is the contract-carried SyncTag
                    // (TASK-0172). It is the wire `barrier_cross`
                    // token, so host and worker must agree on it:
                    // every participant of this barrier carries the
                    // SAME tag by construction, so they do — including
                    // for partial/non-uniform barriers where the old
                    // per-worker pre-order index would have diverged.
                    let bid = sync.0;
                    // Host-mediated star barrier. Host crosses with
                    // every non-host participant (deterministic
                    // WorkerId order); a non-host worker crosses with
                    // host only. 2-party (tier-1) is the trivial
                    // case. The `barrier_cross` helper is
                    // send-then-recv on both ends — safe over a
                    // duplex stream for a 16-byte token.
                    if is_host {
                        let mut peers: Vec<WorkerId> = participants
                            .iter()
                            .copied()
                            .filter(|p| *p != self.host_worker)
                            .collect();
                        peers.sort_unstable();
                        for p in peers {
                            let cv = self.ctrl_var(true, p);
                            writeln!(out, "{pad}wire::barrier_cross(&mut {cv}, {bid});").ok();
                        }
                    } else {
                        writeln!(out, "{pad}wire::barrier_cross(&mut ctrl_host, {bid});").ok();
                    }
                }
                Event::Push { data, dst, seq, .. } => {
                    let _xid = self.xfer_ids.get(data).ok_or_else(|| {
                        EmitError::ContractGap(format!(
                            "Push of data {data:?} not collected as cross-worker"
                        ))
                    })?;
                    let name = self.data_name(*data)?;
                    let ty = self.sidecar.data_type(*data).ok_or_else(|| {
                        EmitError::ContractGap(format!(
                            "cross-worker data `{name}` ({data:?}) has no ResolvedType"
                        ))
                    })?;
                    let enc = encode_expr(&name, ty)?;
                    // The connection to the *destination* worker. The
                    // dst must be a peer of this worker on the
                    // (data,ctrl)-pair-per-(host,worker) topology.
                    let cv = self.data_conn_var(worker, is_host, *dst)?;
                    let to = self.worker_name(*dst);
                    writeln!(
                        out,
                        "{pad}wire::write_msg(&mut {cv}, {}, &{enc}); // send `{name}` to {to}",
                        seq.0
                    )
                    .ok();
                }
                Event::Wait { data, src, seq, .. } => {
                    let _xid = self.xfer_ids.get(data).ok_or_else(|| {
                        EmitError::ContractGap(format!(
                            "Wait of data {data:?} not collected as cross-worker"
                        ))
                    })?;
                    let name = self.data_name(*data)?;
                    let ty = self.sidecar.data_type(*data).ok_or_else(|| {
                        EmitError::ContractGap(format!(
                            "cross-worker data `{name}` ({data:?}) has no ResolvedType"
                        ))
                    })?;
                    let cv = self.data_conn_var(worker, is_host, *src)?;
                    let from = self.worker_name(*src);
                    let dec = decode_expr(ty)?;
                    // TASK-0343 cycle 189: per-(worker, data, seq)
                    // overlapping-write accumulator classification. Same
                    // shape as the shared walker's Wait emit (see
                    // multi_worker_walker/event_walker.rs Event::Wait
                    // branch); mp-tcp-bufsync bypasses the walker but
                    // consumes the same accumulate set populated at
                    // Plan::build time and the same render_wait_assign
                    // helper.
                    let accumulate = self.accumulate_waits.contains(&(worker, *data, *seq));
                    // TASK-0296 cycle 116: route Wait gather through the
                    // shared backend-common slice-paste helper. Before
                    // this, the host-side emit was `{name} = {dec};`
                    // (whole-array overwrite) regardless of the pair's
                    // tile — partition-band gathers silently lost their
                    // slice, e.g. 06-separable-filter/distributed × mp-
                    // tcp-bufsync overwrote `tmp` per recv instead of
                    // pasting each worker's hy row-band. The shared
                    // helper dispatches whole-array vs 1D leading-axis
                    // vs 2D row-loop slice-paste from the IterTile;
                    // pthreads-async + mp-tcp-event already went via
                    // this helper (silent-sibling defect closure for
                    // mp-tcp-bufsync).
                    let assign = render_wait_assign(
                        self.sidecar,
                        &self.pair_tiles,
                        &name,
                        *data,
                        *seq,
                        &dec,
                        accumulate,
                    )?;
                    writeln!(
                        out,
                        "{pad}{{ let __buf = wire::read_msg_expect(&mut {cv}, {}); \
                         {assign} }} // recv `{name}` from {from}",
                        seq.0
                    )
                    .ok();
                }
                Event::Alloc { .. } | Event::Free { .. } => {
                    // RAII Vec storage; no explicit reservation (same
                    // as pthreads-sync).
                }
            }
        }
        Ok(())
    }
}
