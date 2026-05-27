//! Event-tree walkers and pure analyses used by [`super::Plan::build`] +
//! [`super::Plan::relay_schedule`] + [`super::worker_program`].
//!
//! All functions here are pure over `&[Event]` and `&BTreeMap<…>` — no
//! `Plan` self-reference, no codegen. The codegen-using methods on
//! [`super::Plan`] consume the outputs of these walkers (see
//! `Plan::relay_schedule`, `Plan::render_relay_phase`, and the
//! `relay_phase_insertion_point` invocation in
//! `Plan::render_worker_program`).

use std::collections::BTreeMap;

use nucleus_compiler::event::{DataId, Event, SeqTag, WorkerId};

use crate::EmitError;

/// TASK-0332 (cycle 151 AC#2): detect the wait-before-push hazard at
/// codegen time so the synchronous host-relay's circular-seq-dependency
/// deadlock surfaces as a typed `EmitError::ContractGap` instead of a
/// runtime timeout. See the call site in `Plan::build` for the full
/// design narrative; this is the conservative-but-sound implementation.
///
/// Sibling: the same function exists in
/// `nucleus/backends/mp-tcp-bufsync/src/lib.rs` with the same shape +
/// a backend-specific message prefix. Per the cycle-148/149 paired-lift
/// discipline ([[feedback-silent-sibling-defect]] 10th firing), the
/// two implementations were added in the same cycle.
pub(super) fn detect_wait_before_push_hazard(
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
        // Cycle-151 architect P2 note (RESOLVED by TASK-0330): this
        // precondition scans only TOP-LEVEL events for w2w Pushes; the
        // `collect_w2w_pushes` helper recurses into Loop bodies. A
        // worker with Wait at top level + Push inside a Loop body
        // would be a false-negative for THIS detector — but TASK-0330
        // now fires a fail-loud ContractGap in `collect_w2w_pushes` for
        // any w2w Push found INSIDE a Loop body, so the Loop-body
        // hazard is rejected later in the pipeline (in
        // `render_relay_phase` rather than here in `Plan::build`). The
        // composition is sound: a Loop-body w2w Push CANNOT silently
        // reach codegen on either backend.
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
                        "mp-uds-event: worker {w:?} has a worker-to-worker \
                         Wait (from src {src:?}) at top level before any \
                         worker-to-worker Push. Cycle-149's synchronous \
                         host-relay would deadlock on the circular seq \
                         dependency: host's relay blocks at wait(seq) for \
                         this worker's first Push; this worker blocks at \
                         this Wait for host's relay of the seq from \
                         {src:?}. TASK-0332 cycle 151 filed this defensive \
                         guard; TASK-0329.01.01 cycle 162 (Option D) \
                         landed `apply_safe_push_reorder` which hoists \
                         hoistable Pushes ahead of preceding Waits before \
                         this check runs. If you are seeing this guard \
                         fire, the candidate Push was NOT hoistable — \
                         likely because a preceding Fire / Wait / Loop \
                         body writes data the Push depends on, or a \
                         preceding w2w Wait covers an overlapping tile \
                         on a shared axis. See \
                         `nucleus-compiler/src/passes/safe_push_reorder.rs` \
                         for the hoistability predicate."
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

/// Per-pair Push collector: records the (src, dst) of every cross-
/// worker Push. Mirrors `collect_xfer_pairs` but the dst comes from
/// the Push event itself (not from the worker doing the visit).
///
/// **Loop-body interaction with TASK-0330**: this walker recurses into
/// `Event::Loop` bodies and uses `or_insert`, so a w2w Push appearing
/// inside a Loop body would be recorded once (first-visit-wins). The
/// TASK-0330 guard in `collect_w2w_pushes` rejects that shape upstream
/// before host emit reaches `render_relay_phase`, so this walker's
/// duplicate-tolerance is incidentally robust but not load-bearing for
/// the Loop-body case.
pub(super) fn collect_push_pairs(
    events: &[Event],
    src: WorkerId,
    out: &mut BTreeMap<(DataId, SeqTag), (WorkerId, WorkerId)>,
) {
    for e in events {
        match e {
            Event::Push { data, dst, seq, .. } => {
                out.entry((*data, *seq)).or_insert((src, *dst));
            }
            Event::Loop { body, .. } => collect_push_pairs(body, src, out),
            _ => {}
        }
    }
}

/// TASK-0327 (cycle 149): one host-relay hop for a worker-to-worker
/// `Push`/`Wait` pair on the mp-uds-event star topology — "drain
/// `inbound[seq]` from src worker, re-push to `outbound[(seq, dst_peer
/// at host)]` toward dst worker". `data` is for codegen-comment
/// disambiguation only; the wire pass-through is bytes-verbatim.
/// Cap is the chan's per-pair `outbound` bound (`chan_caps`).
#[derive(Debug, Clone, Copy)]
pub(super) struct RelayHop {
    pub(super) seq: SeqTag,
    pub(super) dst: WorkerId,
    pub(super) data: DataId,
    pub(super) cap: u64,
}

/// TASK-0327 (cycle 149) + TASK-0329.01.01 (slice 1): pick the position
/// in HOST's top-level event list at which the host-relay phase should
/// splice in.
///
/// **Cycle 149 design** (scatter-compute-gather): heuristic returned the
/// LAST top-level `Sync` index — splicing relay between pass-1 barrier
/// and pass-2 barrier on schedules like `06-separable-filter/distributed`.
///
/// **Cycle 161/162 update (TASK-0329.01.01 slice 1)**: schedules like
/// `05-stencil/distributed-2d` exchange halo data BEFORE any barrier —
/// after the TASK-0329.01.01 `apply_safe_push_reorder` pass hoists each
/// worker's halo Pushes above its halo Waits, every w2w event happens
/// strictly before the first top-level Sync. The cycle-149 LAST-Sync
/// heuristic placed relay AFTER that Sync, leaving workers blocked at
/// their halo Waits while host blocked at the barrier — observed as a
/// 32s timeout deadlock (cycle 150 empirical finding, TASK-0332).
///
/// The new algorithm walks host's Syncs in event-list order and returns
/// the FIRST Sync (by host's index) such that, for every non-host
/// worker, every w2w event in that worker's event list occurs strictly
/// BEFORE the matching Sync (by `SyncTag` identity — TASK-0172). For
/// schedules where workers' w2w activity is between two barriers (e.g.
/// 06/distributed: w2w activity between bar_0 and bar_1), this still
/// returns bar_1's index — byte-identical with the cycle-149 heuristic.
/// For schedules where workers' w2w activity precedes the first barrier
/// (e.g. 05/distributed-2d after `apply_safe_push_reorder`), this
/// returns bar_0's index, enabling relay to run BEFORE the barrier.
///
/// Constraint 3 of the cycle-149 design (per-seq demux removes the
/// stream-race hazard that bufsync's analogous splice has — see memory
/// `project-mp-uds-event-vs-bufsync-safety-profile`) is still INERT on
/// mp-uds-event: moving the relay earlier in host's events is safe
/// because each `relay_one(seq, ...)` drains a distinct `inbound[seq]`
/// queue. **mp-tcp-bufsync's `relay_phase_insertion_point` is NOT
/// updated** — see AC#3b of TASK-0329.01.01 + the comment at the
/// bufsync splice site.
///
/// Fallbacks (preserving cycle-149 behaviour for schedules where the
/// scan finds nothing): LAST Sync (cycle-149 primary), then FIRST
/// top-level Wait, then end-of-events.
pub(super) fn relay_phase_insertion_point(
    host_events: &[Event],
    per_worker: &BTreeMap<WorkerId, Vec<Event>>,
    host: WorkerId,
) -> usize {
    'outer: for (host_idx, ev) in host_events.iter().enumerate() {
        let Event::Sync { sync: host_tag, .. } = ev else {
            continue;
        };
        for (&w, w_events) in per_worker {
            if w == host {
                continue;
            }
            // Find this worker's matching Sync by SyncTag (TASK-0172).
            let Some(w_sync_idx) = w_events
                .iter()
                .position(|e| matches!(e, Event::Sync { sync, .. } if sync == host_tag))
            else {
                // Worker doesn't participate in this Sync. Not a hard
                // disqualifier — host's barrier shim handles partial
                // barriers via the `barrier_participants` set. Skip
                // this worker for the all-complete check.
                continue;
            };
            // Are there any w2w events at or after this worker's
            // matching Sync? If yes, this host Sync is too early.
            let has_post_w2w = w_events[w_sync_idx..].iter().any(|e| match e {
                Event::Push { dst, .. } if *dst != host => true,
                Event::Wait { src, .. } if *src != host => true,
                _ => false,
            });
            if has_post_w2w {
                continue 'outer;
            }
        }
        // Every non-host worker has finished w2w by this Sync; relay
        // splices at this host Sync index (= relay runs BEFORE this
        // Sync in host's event stream).
        return host_idx;
    }
    // Fallbacks (cycle-149 originals, kept for schedules that don't
    // match the scan — e.g. relay-needed shapes without any top-level
    // Sync would never reach `render_relay_phase` because the schedule
    // would have rejected at `detect_wait_before_push_hazard` first
    // for any non-trivial w2w shape).
    if let Some(idx) = host_events
        .iter()
        .rposition(|e| matches!(e, Event::Sync { .. }))
    {
        return idx;
    }
    if let Some(idx) = host_events.iter().position(|e| matches!(e, Event::Wait { .. })) {
        return idx;
    }
    host_events.len()
}

/// TASK-0327 (cycle 149): collect every Push event where the dst is a
/// non-host worker (= worker-to-worker push), in event-list order.
/// `cap` is looked up against `chan_caps` so the emitted relay code
/// can pass the right back-pressure bound.
///
/// Recurses into Loop bodies in event-list order; the relay block is
/// emitted FLAT outside any loop.
///
/// **TASK-0330 active guard** (defensive ContractGap):
/// when the recursion is INSIDE an `Event::Loop` body and encounters a
/// w2w `Push`, returns [`EmitError::ContractGap`] forward-linking
/// TASK-0330. The flat relay block would either over-count (host calls
/// `relay_one` once per (seq, dst) but the worker pushes N times around
/// the loop) or mis-order (the flat replay order would not align with
/// the loop's nested iteration order). Fail-loud at codegen > silent
/// miscompile, per [[feedback-panic-not-diagnostic-recurring]].
///
/// **TASK-0329.01.02 cycle 163 update + cycle-166 reframe (slice 2
/// lift + residual-class enumeration):** the compiler-level
/// `apply_host_data_relay_inject` ACFG pass
/// (`nucleus-compiler/src/passes/host_data_relay_inject.rs`) routes
/// every PAIRED non-host Push/Wait inside a Repeat body through host
/// BEFORE projection. After the pass, the in-Loop-body w2w-Push shape
/// this guard fires on is structurally impossible for shapes the pass
/// handled. The guard STAYS in place as a fail-loud safety net for
/// two precisely-enumerated residual classes (cycle-163b architect
/// P2.4 → cycle 166 audit confirmed both classes still reachable in
/// principle):
///
/// - **(R-bare)** A bare `Xfer` outside any parent `Sequence`. The
///   pass requires a sibling slot to land the 4 routed nodes into;
///   `rewrite_at`'s non-`Sequence` arm early-returns. In practice
///   every `Xfer` from `transfer_inject` is produced inside a
///   `Sequence` — this class is reachable only if `transfer_inject`'s
///   contract weakens.
/// - **(R-singleton)** A `Push` (or `Wait`) without its matching
///   sibling endpoint in the SAME `Sequence`. The pass's pair-match
///   in `rewrite_sequence_children` requires both endpoints to live
///   in the same `Sequence`; if `transfer_inject::hoist_invariant_waits`
///   has hoisted one endpoint out, the unmatched endpoint is left
///   alone and would reach this guard.
///
/// In-tree schedules today: ZERO residual hits on the post-pass
/// matrix (verified across the 3 trigger cells in cycles 162/163/165
/// and re-verified bit-identical in cycle 166's audit). Test pins in
/// `nucleus/backends/mp-uds-event/tests/loop_body_w2w_push.rs`
/// continue to assert the guard's contract on synthetic
/// pass-bypassing fixtures (those fixtures construct the Loop-body
/// shape directly, never via the pass).
pub(super) fn collect_w2w_pushes(
    events: &[Event],
    host: WorkerId,
    chan_caps: &BTreeMap<(DataId, SeqTag), u64>,
    out: &mut Vec<RelayHop>,
) -> Result<(), EmitError> {
    collect_w2w_pushes_inner(events, host, false, chan_caps, out)
}

fn collect_w2w_pushes_inner(
    events: &[Event],
    host: WorkerId,
    inside_loop: bool,
    chan_caps: &BTreeMap<(DataId, SeqTag), u64>,
    out: &mut Vec<RelayHop>,
) -> Result<(), EmitError> {
    for e in events {
        match e {
            Event::Push { dst, data, seq, .. } if *dst != host => {
                if inside_loop {
                    return Err(EmitError::ContractGap(format!(
                        "mp-uds-event: TASK-0330 defensive guard — \
                         worker-to-worker Push (data={data:?}, dst={dst:?}, \
                         seq={seq:?}) found INSIDE an Event::Loop body. The \
                         cycle-149 host-relay (TASK-0327) emits the relay \
                         block FLAT outside any loop, so a nested w2w Push \
                         would either over-count (host calls relay_one once \
                         per (seq, dst) but the worker pushes N times around \
                         the loop) or mis-order (the flat replay order would \
                         not align with the loop's nested iteration order). \
                         TASK-0329.01.02 cycle 163 (slice 2): the compiler \
                         pass `apply_host_data_relay_inject` routes every \
                         non-host-pair Push/Wait through host at the ACFG \
                         layer; if you are seeing this guard fire on an \
                         in-tree schedule, the pass either didn't run \
                         (driver wiring is mp-uds-event-only — check \
                         driver/src/main.rs near `apply_host_data_relay_inject`) \
                         or the pair predicate didn't fire (Push/Wait not in \
                         the same Sequence — see \
                         `host_data_relay_inject::rewrite_sequence_children` \
                         singleton-left-alone comment for the residual class)."
                    )));
                }
                let cap = chan_caps.get(&(*data, *seq)).copied().ok_or_else(|| {
                    EmitError::ContractGap(format!(
                        "mp-uds-event relay schedule: missing chan_caps for \
                         (data={data:?}, seq={seq:?}) — Push collected but \
                         Plan::build did not populate the cap"
                    ))
                })?;
                out.push(RelayHop {
                    seq: *seq,
                    dst: *dst,
                    data: *data,
                    cap,
                });
            }
            Event::Loop { body, .. } => {
                collect_w2w_pushes_inner(body, host, true, chan_caps, out)?;
            }
            _ => {}
        }
    }
    Ok(())
}
