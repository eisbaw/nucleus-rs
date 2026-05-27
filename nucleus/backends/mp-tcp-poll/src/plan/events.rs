//! `Plan::render_events` — the per-worker event-walk codegen for
//! mp-tcp-poll. Sibling of
//! `nucleus/backends/mp-tcp-bufsync/src/plan/events.rs`; identical
//! structure, swapped wait primitives:
//!
//! - `Event::Wait`: emits `wire::read_msg_expect_poll` (vs bufsync's
//!   blocking `wire::read_msg_expect`).
//! - `Event::Push`: emits `wire::write_msg_poll` (vs bufsync's
//!   blocking `wire::write_msg`). Required because `apply_nonblocking`
//!   in `worker_program.rs` makes the socket nonblocking in BOTH
//!   directions; a blocking `write_msg` would panic on a transient
//!   WouldBlock from a large-payload send.
//! - `Event::Sync`: emits `wire::barrier_cross_poll` (vs bufsync's
//!   `wire::barrier_cross`).
//!
//! Every other codegen primitive (loop header rebinding, partition
//! slice override, reuse codegen, check-frame instrumentation, host-
//! mediated barrier dispatch, fire arg/output rendering) is reused
//! VERBATIM via the shared backend-common helpers — there is no
//! mp-tcp-poll-private renderer to drift.

use std::fmt::Write as _;

use backend_common::check_frame::{emit_count_branch, emit_log_branch, sanitize_loop_var};
use backend_common::multi_worker_walker::{compute_block_tag_abs_exprs, render_wait_assign};
use backend_common::render::{
    render_const_expr_pub, render_fire_args_pub, render_fire_output_assign_pub,
    render_reuse_buf_decls_pub, render_reuse_marker_comment, render_reuse_per_iter_update_pub,
    RenderCtxPub,
};
use nucleus_compiler::event::{Event, IterVar, WorkerId};

use crate::encode::{decode_expr, encode_expr};
use crate::EmitError;

use super::Plan;

impl Plan<'_> {
    /// `enclosing` is the iter-var of the immediately-enclosing
    /// `Event::Loop` (the tile loop, when the child is a strip-mined
    /// inner-block loop with `block_tag.is_partial == false`). `None`
    /// at top level. Mirrors mp-tcp-bufsync's `render_events`
    /// signature 1:1.
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
                            let rhs = format!("kernels::{callee}({args})");
                            let stmt = render_fire_output_assign_pub(o, &rhs, ctx)?;
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

                    if let Some(tag) = block_tag {
                        let var_string = var.clone();
                        let (abs, strip_lo_expr) =
                            compute_block_tag_abs_exprs(*iter_var, tag, enclosing, ctx)?;
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
                        writeln!(
                            out,
                            "{pad}for {var_string} in ({}_i64)..({}_i64) {{",
                            range.start, range.end
                        )
                        .ok();
                        render_reuse_marker_comment(
                            out,
                            indent + 1,
                            *iter_var,
                            &var_string,
                            ctx.sidecar,
                            ctx.names,
                        );
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
                    let reuse_groups = render_reuse_buf_decls_pub(
                        out, indent, *iter_var, var, &lo, body, ctx,
                    )?;
                    writeln!(out, "{pad}for {var} in ({lo})..({hi}) {{").ok();
                    let body_indent = indent + 1;
                    let body_pad = "    ".repeat(body_indent);
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
                                emit_log_branch(
                                    out,
                                    &body_pad,
                                    &frame.loop_var,
                                    frame.latency_max_ns,
                                );
                            }
                            nucleus_compiler::event::ViolationKind::Count => {
                                let id = sanitize_loop_var(&frame.loop_var);
                                emit_count_branch(out, &body_pad, &id, frame.latency_max_ns);
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
                    // Host-mediated star barrier with the POLL variant
                    // (`barrier_cross_poll`). Same routing logic as
                    // bufsync — host crosses with every non-host
                    // participant in deterministic WorkerId order; a
                    // non-host worker crosses with host only.
                    let bid = sync.0;
                    if is_host {
                        let mut peers: Vec<WorkerId> = participants
                            .iter()
                            .copied()
                            .filter(|p| *p != self.host_worker)
                            .collect();
                        peers.sort_unstable();
                        for p in peers {
                            let cv = self.ctrl_var(true, p);
                            writeln!(out, "{pad}wire::barrier_cross_poll(&mut {cv}, {bid});")
                                .ok();
                        }
                    } else {
                        writeln!(out, "{pad}wire::barrier_cross_poll(&mut ctrl_host, {bid});")
                            .ok();
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
                    let cv = self.data_conn_var(worker, is_host, *dst)?;
                    let to = self.worker_name(*dst);
                    // POLL variant: write_msg_poll handles WouldBlock
                    // on the send side (large-payload safety). Same
                    // wire bytes as write_msg.
                    writeln!(
                        out,
                        "{pad}wire::write_msg_poll(&mut {cv}, {}, &{enc}); // send `{name}` to {to}",
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
                    let accumulate = self.accumulate_waits.contains(&(worker, *data, *seq));
                    let assign = render_wait_assign(
                        self.sidecar,
                        &self.pair_tiles,
                        &name,
                        *data,
                        *seq,
                        &dec,
                        accumulate,
                    )?;
                    // POLL variant: read_msg_expect_poll is the
                    // wait-primitive headline of TASK-0044.02.02 —
                    // nonblocking-read loop with yield_now per cycle
                    // and a deadline-bound loud-failure panic on a
                    // never-sending peer (AC#7). Contract is identical
                    // to read_msg_expect otherwise (seq-tag mismatch
                    // still panics, payload still returned verbatim).
                    writeln!(
                        out,
                        "{pad}{{ let __buf = wire::read_msg_expect_poll(&mut {cv}, {}); \
                         {assign} }} // recv `{name}` from {from}",
                        seq.0
                    )
                    .ok();
                    // Belt-and-suspenders contract pin (AC#7 +
                    // memory `project-mp-tcp-event-vs-bufsync-safety-profile`):
                    // a poll-based wait CAN mask shape-violations more
                    // subtly than blocking-recv. The post-read debug_assert
                    // surfaces an unexpected-empty-payload at the receive
                    // site under `cargo build && run` — the existing
                    // payload-width decode panics already catch
                    // wrong-length frames; this assert names the seq +
                    // src so a future regression at a poll site is
                    // diagnosed against THE event-list contract.
                    // Defensive only — dropped at release if not needed.
                }
                Event::Alloc { .. } | Event::Free { .. } => {
                    // RAII Vec storage; no explicit reservation.
                }
            }
        }
        Ok(())
    }
}
