//! Event-walk helpers for mp-tcp-poll (multi-process arm).
//!
//! Sibling of `nucleus/backends/mp-tcp-bufsync/src/walkers.rs`
//! — copied verbatim at cycle 195 (TASK-0044.02.02) because the
//! analysis logic (xfer-data collection, w2w-push collection, FIFO-
//! shape hazards) is wait-primitive-agnostic. The only difference
//! between the two backends is the wait primitive used in the emitted
//! code (`wire::read_msg_expect_poll` here vs blocking
//! `wire::read_msg_expect` on bufsync); the same per-pair FIFO
//! topology + the same wait-before-push deadlock surface apply
//! identically on poll because nonblocking-read does NOT change the
//! ordering of frames on the wire — it only changes how the receiver
//! waits for them (poll-then-yield vs blocking-recv).
//!
//! Follow-up task TASK-0044.02.02-followup-shared-plan-crate carries
//! the eventual lift of this duplicated substrate to a shared crate.

use std::collections::{BTreeMap, BTreeSet};

use nucleus_compiler::event::{DataId, Event, SeqTag, SyncTag, WorkerId};

use crate::EmitError;

/// One host-relay hop = "read seq N from data_<src>, write seq N to
/// data_<dst>". `data` is the DataId for codegen comment only; the
/// wire pass-through is bytes-verbatim. Same shape as
/// mp-tcp-bufsync's `RelayHop` (TASK-0327 cycle 148).
#[derive(Debug, Clone, Copy)]
pub(crate) struct RelayHop {
    pub(crate) seq: SeqTag,
    pub(crate) dst: WorkerId,
    pub(crate) data: DataId,
}

/// **Loop-body interaction with TASK-0330**: this walker recurses into
/// `Event::Loop` bodies and accumulates DataIds via a set (`insert`),
/// so a Loop-body Push/Wait is idempotent across iterations and benign
/// here regardless of TASK-0330's guard. The TASK-0330 guard rejects
/// the w2w-Push-in-Loop shape upstream in `collect_w2w_pushes` before
/// it would matter; this walker's set-union shape is incidentally
/// robust independently.
pub(crate) fn collect_xfer_data(events: &[Event], out: &mut BTreeSet<DataId>) {
    for e in events {
        match e {
            Event::Push { data, .. } | Event::Wait { data, .. } => {
                out.insert(*data);
            }
            Event::Loop { body, .. } => collect_xfer_data(body, out),
            _ => {}
        }
    }
}

/// Pick the position in HOST's top-level event list at which the
/// host-relay phase should splice in. Same heuristic as
/// mp-tcp-bufsync's sibling (cycle 148): primary = just before the
/// LAST top-level `Event::Sync` (satisfies the pass-1/pass-2 barrier
/// constraints); fallback = before the first top-level `Event::Wait`
/// (gather start, satisfies the FIFO race constraint when no Sync
/// anchor exists); last resort = end.
///
/// mp-tcp-poll inherits the same per-pair FIFO single-stream
/// constraint as bufsync (per-seq demux is mp-tcp-EVENT only), so the
/// mp-tcp-event-specific "walk by SyncTag" optimisation in
/// `nucleus/backends/mp-tcp-event/src/multi_worker.rs` is NOT used
/// here.
pub(crate) fn relay_phase_insertion_point(events: &[Event]) -> usize {
    if let Some(idx) = events.iter().rposition(|e| matches!(e, Event::Sync { .. })) {
        return idx;
    }
    if let Some(idx) = events.iter().position(|e| matches!(e, Event::Wait { .. })) {
        return idx;
    }
    events.len()
}

/// Collect every Push event where the dst is a non-host worker —
/// these are the w2w pushes that host must relay. Recurses into Loop
/// bodies in event-list order; the relay block is emitted FLAT
/// outside any loop.
///
/// **TASK-0330 active guard** (defensive ContractGap): when the
/// recursion is INSIDE an `Event::Loop` body and encounters a w2w
/// `Push`, returns [`EmitError::ContractGap`] forward-linking
/// TASK-0330 (same rationale as the mp-tcp-bufsync sibling: the flat
/// relay block would either over-count or mis-order). Fail-loud at
/// codegen > silent miscompile.
///
/// In-tree mp-tcp-poll schedules in the cycle-195 promotion wave
/// (02-split, 03-reduction/distributed, 06/distributed,
/// 06/distributed2, 07/distributed, 07/distributed-2d, 08/distributed,
/// 13/batch_parallel) all keep w2w Pushes at top level — the same
/// shape as mp-tcp-bufsync's promoted cells — so this guard is
/// dormant on the current matrix; it pins the contract for a future
/// schedule shape. mp-tcp-event's compiler-pass remediation
/// (`apply_host_data_relay_inject`) is wired ONLY for mp-tcp-event
/// per the per-pair FIFO constraint that makes the splice-point lift
/// unsafe on bufsync/poll (memory
/// `project-mp-tcp-event-vs-bufsync-safety-profile`).
pub(crate) fn collect_w2w_pushes(
    events: &[Event],
    host: WorkerId,
    out: &mut Vec<RelayHop>,
) -> Result<(), EmitError> {
    collect_w2w_pushes_inner(events, host, false, out)
}

fn collect_w2w_pushes_inner(
    events: &[Event],
    host: WorkerId,
    inside_loop: bool,
    out: &mut Vec<RelayHop>,
) -> Result<(), EmitError> {
    for e in events {
        match e {
            Event::Push { dst, data, seq, .. } if *dst != host => {
                if inside_loop {
                    return Err(EmitError::ContractGap(format!(
                        "mp-tcp-poll: TASK-0330 defensive guard — \
                         worker-to-worker Push (data={data:?}, dst={dst:?}, \
                         seq={seq:?}) found INSIDE an Event::Loop body. The \
                         cycle-195 host-relay (sibling of bufsync's TASK-0327 \
                         cycle-148 emit) writes the relay block FLAT outside \
                         any loop, so a nested w2w Push would either over-count \
                         or mis-order. No in-tree schedule trips this today. \
                         mp-tcp-event carries the compiler-pass remediation \
                         (`apply_host_data_relay_inject`); mp-tcp-poll inherits \
                         bufsync's per-pair FIFO constraint and so the pass is \
                         NOT wired here. If a future poll-capable schedule \
                         needs the equivalent, file a follow-up — the pass \
                         itself is backend-agnostic at the ACFG layer."
                    )));
                }
                out.push(RelayHop {
                    seq: *seq,
                    dst: *dst,
                    data: *data,
                });
            }
            Event::Loop { body, .. } => {
                collect_w2w_pushes_inner(body, host, true, out)?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// Detect the wait-before-push hazard at codegen time so the
/// synchronous host-relay's circular-seq-dependency deadlock surfaces
/// as a typed `EmitError::ContractGap` instead of a runtime timeout.
/// Same shape as mp-tcp-bufsync's sibling (TASK-0332 cycle 151) with
/// a backend-specific message prefix.
///
/// On mp-tcp-poll the deadlock surface is conceptually identical to
/// bufsync's because the wire ordering is FIFO per-pair (nonblocking
/// vs blocking read doesn't change the order of frames on the wire —
/// only how the receiver waits for them). The mp-tcp-event-only
/// `apply_safe_push_reorder` lift is NOT applied here (driver gate
/// stays mp-tcp-event-only); the guard rejects any wait-before-push
/// shape unconditionally on mp-tcp-poll, same as on bufsync.
pub(crate) fn detect_wait_before_push_hazard(
    per_worker: &BTreeMap<WorkerId, Vec<Event>>,
    host: WorkerId,
) -> Result<(), EmitError> {
    for (&w, events) in per_worker {
        if w == host {
            continue;
        }
        let has_w2w_push = events
            .iter()
            .any(|e| matches!(e, Event::Push { dst, .. } if *dst != host));
        if !has_w2w_push {
            continue;
        }
        for e in events {
            match e {
                Event::Push { dst, .. } if *dst != host => break,
                Event::Wait { src, .. } if *src != host => {
                    return Err(EmitError::ContractGap(format!(
                        "mp-tcp-poll: worker {w:?} has a worker-to-worker \
                         Wait (from src {src:?}) at top level before any \
                         worker-to-worker Push. Cycle-195's synchronous \
                         host-relay would deadlock on the circular seq \
                         dependency (sibling of bufsync's TASK-0332 cycle \
                         151 guard). The push-before-wait reorder pass \
                         (apply_safe_push_reorder) is wired on mp-tcp-event \
                         ONLY — mp-tcp-poll inherits bufsync's per-pair FIFO \
                         constraint that makes the splice-point lift unsafe \
                         here. If a future capability lift exposes a \
                         poll-compatible wait-before-push schedule, a \
                         backend-specific architectural fix would be needed."
                    )));
                }
                _ => continue,
            }
        }
    }
    Ok(())
}

/// Sync visitor: invoke `f(sync_tag, participants)` for each
/// `Event::Sync`, descending into Loop bodies. Barrier identity is
/// the contract-carried [`SyncTag`] (TASK-0172) — no running index.
pub(crate) fn collect_barriers_by_tag<F>(events: &[Event], f: &mut F)
where
    F: FnMut(SyncTag, &BTreeSet<WorkerId>),
{
    for e in events {
        match e {
            Event::Sync {
                participants, sync, ..
            } => f(*sync, participants),
            Event::Loop { body, .. } => collect_barriers_by_tag(body, f),
            _ => {}
        }
    }
}
