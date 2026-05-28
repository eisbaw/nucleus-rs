//! Event-walk analysis helpers shared by the sync-TCP multi-process
//! backends (mp-tcp-bufsync, mp-tcp-poll). These walk the per-worker
//! `Event` lists to derive cross-worker structure (xfer data, w2w
//! relay hops, barrier participants) and to fail-loud on schedule
//! shapes the synchronous host-relay cannot lower.
//!
//! Wire-primitive-agnostic in BEHAVIOUR: nonblocking-read does NOT
//! change the order of frames on the wire — it only changes how the
//! receiver waits for them — so the FIFO-shape hazards apply
//! identically to both backends. The ONLY per-backend variation is
//! the `EmitError::ContractGap` MESSAGE PREFIX (the backend name),
//! routed through [`WirePrimitives::BACKEND_NAME`]. These messages are
//! compiler-time diagnostics, NOT text emitted into generated code.
//!
//! Lifted from the two backends' verbatim-duplicate `walkers.rs`
//! (TASK-0044.02.03). `RelayHop` is the host-relay descriptor (TASK-
//! 0327): produced by [`collect_w2w_pushes`], consumed by the shared
//! `Plan::render_relay_phase`.

use std::collections::{BTreeMap, BTreeSet};

use nucleus_compiler::event::{DataId, Event, SeqTag, SyncTag, WorkerId};

use crate::tcp_plan::WirePrimitives;
use crate::EmitError;

/// TASK-0327 (cycle 148): one host-relay hop = "read seq N from
/// data_<src>, write seq N to data_<dst>". `data` is the DataId for
/// codegen comment only; the wire pass-through is bytes-verbatim.
#[derive(Debug, Clone, Copy)]
pub struct RelayHop {
    pub seq: SeqTag,
    pub dst: WorkerId,
    pub data: DataId,
}

/// **Loop-body interaction with TASK-0330**: this walker recurses into
/// `Event::Loop` bodies and accumulates DataIds via a set (`insert`),
/// so a Loop-body Push/Wait is idempotent across iterations and benign
/// here regardless of TASK-0330's guard. The TASK-0330 guard rejects
/// the w2w-Push-in-Loop shape upstream in `collect_w2w_pushes` before
/// it would matter; this walker's set-union shape is incidentally
/// robust independently.
pub fn collect_xfer_data(events: &[Event], out: &mut BTreeSet<DataId>) {
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

/// TASK-0327 (cycle 148): pick the position in HOST's top-level
/// event list at which the host-relay phase should splice in.
///
/// Constraints driving the choice (the 06/distributed2 shape):
///
/// 1. Workers reach their pass-2-end barrier only AFTER receiving
///    their cross-tmps (which require relay) and computing pass 2.
///    So relay must happen BEFORE host's LAST top-level
///    `Event::Sync` — otherwise host blocks at that barrier waiting
///    for workers whose progress is gated on the relay we haven't
///    run yet (circular wait = deadlock).
///
/// 2. Workers reach their pass-1-end barrier (typically the FIRST
///    `Event::Sync` on workers) BEFORE pushing their tmps; so relay
///    needs the workers to have crossed that barrier, which means
///    host must have crossed it too — i.e. relay AFTER host's first
///    Sync (if any) is OK and required.
///
/// 3. (Sub-fallback constraint, only when no top-level Sync exists)
///    Relay reads from `data_<src>` would race host's own reads on
///    the same socket, so relay must happen BEFORE any host Wait
///    on a worker that also has w2w pushes. In practice (with no
///    Sync to anchor on): before the first top-level Wait.
///
/// Heuristic resolution — priority order:
///
/// - **Primary**: insert just BEFORE the LAST top-level
///   `Event::Sync` (= relay happens between the pass-1 barrier and
///   the pass-2 barrier — satisfies constraints 1 + 2). Picked
///   for any schedule whose host events contain >= 1 top-level
///   `Sync`; 06/distributed2 lands here (two top-level Syncs).
/// - **Fallback** (no top-level Sync exists): insert just BEFORE
///   the first top-level `Event::Wait` (the gather start —
///   satisfies constraint 3 alone, which is sufficient when there
///   is no barrier to consider).
/// - **Last resort** (no Sync, no Wait): insert at end.
///
/// "Top-level" = not nested in an `Event::Loop`. The implementation
/// uses `rposition` for the primary (last-Sync) and `position` for
/// the fallback (first-Wait), reflecting the priority above.
///
/// ## Backend asymmetry NOT applied here
///
/// The sibling `nucleus/backends/mp-tcp-event/src/multi_worker.rs`
/// `relay_phase_insertion_point` was updated (TASK-0329.01.01 slice 1)
/// to walk worker events by `SyncTag` and return the FIRST Sync after
/// which every non-host worker has finished w2w activity. That change
/// is mp-tcp-event-only: on mp-tcp-event constraint 3 above is INERT
/// (per-seq demux removes the stream-race hazard), so the relay can
/// splice before host's own w2w Waits without a race. The two
/// sync-TCP backends here use one ordered DATA stream per `(host,
/// worker)` pair — moving the relay earlier would race host's own
/// reads on `data_<src>` (constraint 3 ACTIVE). Per memory
/// `project-mp-tcp-event-vs-bufsync-safety-profile` the per-seq vs
/// FIFO distinction is load-bearing for this asymmetry.
pub fn relay_phase_insertion_point(events: &[Event]) -> usize {
    if let Some(idx) = events.iter().rposition(|e| matches!(e, Event::Sync { .. })) {
        return idx;
    }
    if let Some(idx) = events.iter().position(|e| matches!(e, Event::Wait { .. })) {
        return idx;
    }
    events.len()
}

/// TASK-0327 (cycle 148): collect every Push event where the dst is
/// a non-host worker — these are the w2w pushes that host must relay.
/// Recurses into Loop bodies in event-list order; the relay block is
/// emitted FLAT outside any loop.
///
/// **TASK-0330 active guard** (defensive ContractGap):
/// when the recursion is INSIDE an `Event::Loop` body and encounters a
/// w2w `Push`, returns [`EmitError::ContractGap`] forward-linking
/// TASK-0330. The flat relay block would either over-count (host reads
/// once per (seq, dst) but the worker pushes N times around the loop)
/// or mis-order (the flat read order would not align with the loop's
/// nested iteration order). Fail-loud at codegen > silent miscompile or
/// runtime deadlock, per [[feedback-panic-not-diagnostic-recurring]].
///
/// In-tree schedules today have all w2w Pushes at TOP LEVEL on both
/// sync-TCP backends, so this guard is dormant on the current matrix;
/// it pins the contract for a future schedule shape. The mp-tcp-event
/// sibling carries a compiler-pass remediation
/// (`apply_host_data_relay_inject`) wired only for mp-tcp-event per the
/// per-pair FIFO constraint that makes splice-point lift unsafe on the
/// sync-TCP backends (see memory
/// `project-mp-tcp-event-vs-bufsync-safety-profile`).
pub fn collect_w2w_pushes<W: WirePrimitives>(
    events: &[Event],
    host: WorkerId,
    out: &mut Vec<RelayHop>,
) -> Result<(), EmitError> {
    collect_w2w_pushes_inner::<W>(events, host, false, out)
}

fn collect_w2w_pushes_inner<W: WirePrimitives>(
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
                        "{backend}: TASK-0330 defensive guard — \
                         worker-to-worker Push (data={data:?}, dst={dst:?}, \
                         seq={seq:?}) found INSIDE an Event::Loop body. The \
                         cycle-148 host-relay (TASK-0327) emits the relay \
                         block FLAT outside any loop, so a nested w2w Push \
                         would either over-count (host reads once per \
                         (seq, dst) but the worker pushes N times around \
                         the loop) or mis-order (the flat read order would \
                         not align with the loop's nested iteration order). \
                         No in-tree schedule trips this today. The \
                         mp-tcp-event sibling carries a compiler-pass \
                         remediation (`apply_host_data_relay_inject`, \
                         TASK-0329.01.02 cycle 163 + TASK-0329.01.02.01 \
                         cycle 165) wired only for mp-tcp-event per the \
                         per-pair FIFO constraint that makes splice-point \
                         lift unsafe on the sync-TCP backends (see memory \
                         `project-mp-tcp-event-vs-bufsync-safety-profile`). \
                         If a future sync-TCP-capable schedule needs the \
                         equivalent, file a follow-up; the pass itself is \
                         backend-agnostic at the ACFG layer and would only \
                         require driver-side wiring + a fresh FIFO audit.",
                        backend = W::BACKEND_NAME,
                    )));
                }
                out.push(RelayHop {
                    seq: *seq,
                    dst: *dst,
                    data: *data,
                });
            }
            Event::Loop { body, .. } => {
                collect_w2w_pushes_inner::<W>(body, host, true, out)?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// TASK-0332 (cycle 151 AC#2): detect the wait-before-push hazard at
/// codegen time so the synchronous host-relay's circular-seq-dependency
/// deadlock surfaces as a typed `EmitError::ContractGap` instead of a
/// runtime timeout. See the call site in `Plan::build` for the full
/// design narrative; this is the conservative-but-sound implementation.
///
/// On both sync-TCP backends the deadlock surface is identical because
/// the wire ordering is FIFO per-pair (nonblocking vs blocking read
/// does not change the order of frames on the wire — only how the
/// receiver waits). The mp-tcp-event-only `apply_safe_push_reorder`
/// lift is NOT applied here; the guard rejects any wait-before-push
/// shape unconditionally on both sync-TCP backends.
pub fn detect_wait_before_push_hazard<W: WirePrimitives>(
    per_worker: &BTreeMap<WorkerId, Vec<Event>>,
    host: WorkerId,
) -> Result<(), EmitError> {
    for (&w, events) in per_worker {
        if w == host {
            continue;
        }
        // Precondition: this worker must have at least one w2w Push
        // for the deadlock cycle to involve it. A "pure consumer"
        // worker (only w2w Waits, no w2w Pushes) is NOT a src in
        // `Plan::relay_schedule`, so host's relay does not wait FOR
        // it, so this worker's wait-before-anything pattern cannot
        // close a deadlock cycle from its own side. Pure-consumer
        // workers are SAFE under host-relay.
        //
        // This precondition scans only TOP-LEVEL events for w2w
        // Pushes; the `collect_w2w_pushes` helper recurses into Loop
        // bodies. A worker with Wait at top level + Push inside a Loop
        // body would be a false-negative for THIS detector — but
        // TASK-0330 fires a fail-loud ContractGap in
        // `collect_w2w_pushes` for any w2w Push found INSIDE a Loop
        // body, so the Loop-body hazard is rejected later in the
        // pipeline. A Loop-body w2w Push CANNOT silently reach codegen.
        let has_w2w_push = events
            .iter()
            .any(|e| matches!(e, Event::Push { dst, .. } if *dst != host));
        if !has_w2w_push {
            continue;
        }
        for e in events {
            match e {
                // First top-level w2w event is a Push — safe shape;
                // host's relay can drain this worker's outbound first.
                Event::Push { dst, .. } if *dst != host => break,
                // First top-level w2w event is a Wait — hazard shape;
                // host's relay would deadlock waiting for THIS worker's
                // Push (which the worker can't reach because it's
                // blocked at this Wait).
                Event::Wait { src, .. } if *src != host => {
                    return Err(EmitError::ContractGap(format!(
                        "{backend}: worker {w:?} has a worker-to-worker \
                         Wait (from src {src:?}) at top level before any \
                         worker-to-worker Push. Cycle-148's synchronous \
                         host-relay would deadlock on the circular seq \
                         dependency: host's read blocks for this worker's \
                         first Push; this worker blocks at this Wait for \
                         host's relay of the seq from {src:?}. TASK-0332 \
                         cycle 151 filed this defensive guard. Note: the \
                         push-before-wait reorder pass \
                         (apply_safe_push_reorder, TASK-0329.01.01 slice-1 \
                         Option D) is wired on mp-tcp-event ONLY — the \
                         sync-TCP backends' per-pair FIFO constraint 3 (per \
                         cycle-148 design + memory \
                         `project-mp-tcp-event-vs-bufsync-safety-profile`) \
                         makes the splice-point lift unsafe here, so the \
                         reorder pass cannot be enabled. If a future \
                         capability lift exposes a sync-TCP-compatible \
                         wait-before-push schedule, a backend-specific \
                         architectural fix would be needed.",
                        backend = W::BACKEND_NAME,
                    )));
                }
                // Non-w2w events (Push/Wait with host as the other
                // endpoint, Fire, Sync, Loop, Alloc, Free) don't
                // affect the hazard. Loop bodies are intentionally
                // NOT walked — the hazard is about top-level event
                // order, not nested.
                _ => continue,
            }
        }
    }
    Ok(())
}

/// Sync visitor: invoke `f(sync_tag, participants)` for each
/// `Event::Sync`, descending into Loop bodies. Barrier identity is
/// the contract-carried [`SyncTag`] (TASK-0172) — no running index,
/// no fallibility (every tag is an independent barrier; nothing to
/// validate / reject here any more).
pub fn collect_barriers_by_tag<F>(events: &[Event], f: &mut F)
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
