//! Event-walk helpers (recurse into Loop bodies) — same shapes as
//! pthreads-sync's multi_worker walkers (kept here because the
//! transport-specific Plan differs; the *expression* rendering is the
//! shared part and is NOT re-implemented).
//!
//! Originally inline in `lib.rs` before the slice-4 split.
//!
//! `RelayHop` is the cycle-148 host-relay descriptor (TASK-0327). It is
//! *produced* by `collect_w2w_pushes` (walker) and *consumed* by
//! `plan::relay::Plan::render_relay_phase` (codegen). Co-located here
//! with its producer rather than with its consumer because the producer
//! is the only construction site; the consumer reads a borrowed view.

use std::collections::{BTreeMap, BTreeSet};

use nucleus_compiler::event::{DataId, Event, SeqTag, SyncTag, WorkerId};

use crate::EmitError;

/// TASK-0327 (cycle 148): one host-relay hop = "read seq N from
/// data_<src>, write seq N to data_<dst>". `data` is the DataId for
/// codegen comment only; the wire pass-through is bytes-verbatim.
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
/// Acceptable cycle-148 limitation: any schedule with a host
/// `Sync`-or-`Wait`-AFTER-the-w2w-relay-window structure that does
/// not match this heuristic would deadlock or race; the 06/
/// distributed2 reproducer + the existing 02-split (no w2w) cell +
/// the 03-reduction/distributed cell (no w2w — blocked on
/// host-excluding-barrier, separate gap) all satisfy it.
///
/// ## TASK-0329.01.01 (slice 1) — backend asymmetry NOT applied here
///
/// The sibling `nucleus/backends/mp-tcp-event/src/multi_worker.rs`
/// `relay_phase_insertion_point` was updated in slice 1 of
/// TASK-0329.01.01 to walk worker events by `SyncTag` and return the
/// FIRST Sync after which every non-host worker has finished w2w
/// activity. That change is mp-tcp-event-only: on mp-tcp-event,
/// constraint 3 above is INERT (per-seq demux removes the
/// stream-race hazard), so the relay can splice before host's own
/// w2w Waits without a race. Bufsync uses one ordered DATA stream
/// per `(host, worker)` pair — moving the relay earlier here would
/// race host's own reads on `data_<src>` (constraint 3 ACTIVE). The
/// 05/distributed-2d wait-before-push hazard would, on bufsync, need
/// either a threaded relay or a per-pair-multiplex change to the
/// wire codec — neither in scope for slice 1. Per memory
/// `project-mp-tcp-event-vs-bufsync-safety-profile` the per-seq vs
/// FIFO distinction is load-bearing for this asymmetry.
pub(crate) fn relay_phase_insertion_point(events: &[Event]) -> usize {
    if let Some(idx) = events
        .iter()
        .rposition(|e| matches!(e, Event::Sync { .. }))
    {
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
/// In-tree schedules today have all w2w Pushes at TOP LEVEL — verified
/// by `host_relay_emit` and the cycle-148 reviewer audit — so this
/// guard is dormant on the current matrix; it pins the contract for a
/// future schedule shape, with test pins in
/// `nucleus/backends/mp-tcp-bufsync/tests/loop_body_w2w_push.rs`.
///
/// **TASK-0329.01.02 cycle 163 (slice 2) AC#5 bufsync audit + cycle-166
/// reframe — guard stays as-is on bufsync; pass NOT mirrored:**
/// the compiler-level `apply_host_data_relay_inject` pass that lifts
/// this guard on mp-tcp-event (sibling backend) is intentionally NOT
/// wired into the driver for mp-tcp-bufsync. Reasoning:
/// (a) mp-tcp-bufsync's 09/13 cells are capability-gated on
///     async/buffer/event so behavioral verification of any pass
///     effect on bufsync would be impossible (capability-skip happens
///     BEFORE codegen);
/// (b) bufsync's per-pair FIFO single stream + `wire::read_msg_expect`
///     panic-on-seq-mismatch (memory
///     `project-mp-tcp-event-vs-bufsync-safety-profile`) has a
///     different failure profile than mp-tcp-event's per-seq-demux;
///     enabling the pass on bufsync without a runtime verification
///     path is a defensible-gain-of-zero risk.
///
/// **Residual safety-net scope (cycle 166 paired with mp-tcp-event
/// sibling).** Because the pass is NOT enabled on bufsync, this
/// guard's reachable shape set is BROADER than the sibling's: every
/// Loop-body w2w `Push` reaches this guard, whereas on mp-tcp-event
/// only the cycle-163b residual classes do. For cross-backend
/// vocabulary parity (so a future reviewer can grep both sibling
/// docstrings consistently), those classes are:
/// - **(R-bare)** A bare `Xfer` outside any parent `Sequence` (would
///   only matter on this backend if the pass were eventually enabled
///   here AND `transfer_inject`'s contract weakened).
/// - **(R-singleton)** A `Push`/`Wait` without its matching sibling
///   endpoint in the same `Sequence` (same conditional applies).
///
/// On bufsync today the operative class is simply "any Loop-body w2w
/// Push" — the residuals (R-bare)/(R-singleton) become operative only
/// if a future cycle enables the pass here.
///
/// **Affirmative structural finding (cycle-163b architect P2.1
/// fold-back):** the B2 rewrite splits one non-host pair `(w_src,
/// w_dst)` into two pairs `(w_src, host)` and `(host, w_dst)`. Each
/// resulting hop is a single-pair stream with its own monotonically-
/// allocated `seq` (from `max_existing_seq + 1`). The per-pair
/// FIFO invariant `wire::read_msg_expect` relies on is therefore
/// preserved per resulting hop — the pass does NOT introduce a
/// latent seq-mismatch panic surface on future capability-compatible
/// schedules. Skipping the pass on bufsync today is a
/// gain-of-zero-for-cells-that-can't-run risk-mitigation choice, not
/// a "the pass would corrupt bufsync" structural barrier.
///
/// If a future cycle relaxes bufsync's capability gate (or the
/// async/buffer/event semantics are mirrored to a poll/sync transport),
/// re-evaluate whether to enable `apply_host_data_relay_inject` on
/// bufsync. The pass itself is backend-agnostic — only the driver
/// wiring is conditional.
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
                        "mp-tcp-bufsync: TASK-0330 defensive guard — \
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
                         lift unsafe on bufsync (see memory \
                         `project-mp-tcp-event-vs-bufsync-safety-profile`). \
                         If a future bufsync-capable schedule needs the \
                         equivalent, file a follow-up; the pass itself is \
                         backend-agnostic at the ACFG layer and would only \
                         require driver-side wiring + a fresh FIFO audit."
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

/// TASK-0332 (cycle 151 AC#2): detect the wait-before-push hazard at
/// codegen time so the synchronous host-relay's circular-seq-dependency
/// deadlock surfaces as a typed `EmitError::ContractGap` instead of a
/// runtime timeout. See the call site in `Plan::build` for the full
/// design narrative; this is the conservative-but-sound implementation.
///
/// Sibling: the same function exists in
/// `nucleus/backends/mp-tcp-event/src/multi_worker.rs` with the same
/// shape + a backend-specific message prefix. Per the cycle-148/149
/// paired-lift discipline ([[feedback-silent-sibling-defect]] 10th
/// firing), the two implementations were added in the same cycle.
///
/// Cycle-151 architect P1 fold-back: this function was originally
/// placed BEFORE `collect_w2w_pushes` with no blank-line separator,
/// which caused `collect_w2w_pushes`'s cycle-148 docstring to be
/// silently absorbed into this docstring (a paired-lift sibling-
/// defect — the mp-tcp-event sibling avoided it by accident of file
/// structure). Folded back by moving this function AFTER
/// `collect_w2w_pushes`, restoring the docstring boundary.
pub(crate) fn detect_wait_before_push_hazard(
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
                        "mp-tcp-bufsync: worker {w:?} has a worker-to-worker \
                         Wait (from src {src:?}) at top level before any \
                         worker-to-worker Push. Cycle-148's synchronous \
                         host-relay would deadlock on the circular seq \
                         dependency: host's wire::read_msg_expect blocks \
                         for this worker's first Push; this worker blocks \
                         at this Wait for host's relay of the seq from \
                         {src:?}. TASK-0332 cycle 151 filed this defensive \
                         guard. Note: TASK-0329.01.01 (slice-1 Option D \
                         push-before-wait reorder) is wired on mp-tcp-event \
                         ONLY — bufsync's per-pair FIFO constraint 3 (per \
                         cycle-148 design + memory \
                         `project-mp-tcp-event-vs-bufsync-safety-profile`) \
                         makes the splice-point lift unsafe on this \
                         backend, so the reorder pass cannot be enabled \
                         here. If a future capability lift exposes a \
                         bufsync-compatible wait-before-push schedule, a \
                         backend-specific architectural fix would be \
                         needed."
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

pub(crate) fn collect_pre_init_sets(
    events: &[Event],
    waited: &mut BTreeSet<DataId>,
    whole: &mut BTreeSet<DataId>,
    indexed: &mut BTreeSet<DataId>,
) {
    for e in events {
        match e {
            Event::Wait { data, .. } => {
                waited.insert(*data);
            }
            Event::Fire { bindings, .. } => {
                if let Some(o) = &bindings.output {
                    if o.indices.is_empty() {
                        whole.insert(o.data);
                    } else {
                        indexed.insert(o.data);
                    }
                }
            }
            Event::Loop { body, .. } => collect_pre_init_sets(body, waited, whole, indexed),
            _ => {}
        }
    }
}
