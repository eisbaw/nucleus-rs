//! The staged-release boot-order fixpoint: compute the `.resc` machine
//! release order so RX is enabled before any peer transmits (TASK-0049.05.03
//! / TASK-0450 split).

use std::collections::{BTreeMap, BTreeSet};

use nucleus_compiler::event::{Event, WorkerId};

/// One worker's first "frame" of cross-MCU link actions, flattened to the
/// ordered list a boot-order simulation needs: a `Recv(seq)` blocks until the
/// producer of `seq` has issued its matching `Push(seq)`; a `Push(seq, dst)`
/// is non-blocking BUT its bytes are DROPPED if `dst` has not yet enabled RX
/// (i.e. not yet been released — TASK-0049.01 `UARTBase.WriteChar` drop).
///
/// We model only the FIRST loop iteration: every subsequent frame repeats the
/// same Push/Recv shape, and boot-order correctness is decided entirely by
/// frame 0 (the bytes that can be dropped before all machines are up are the
/// frame-0 bytes; once every machine is released, RX is on everywhere and no
/// further drops occur). The flattening recurses into `Event::Loop` bodies but
/// does NOT replay the loop range — one pass over the body is the frame shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkAction {
    Recv { seq: u64 },
    Push { seq: u64, dst: WorkerId },
}

/// Flatten a worker's event tree to its ordered `LinkAction` frame (Push/Wait
/// only, recursing into loop bodies in order, one pass per loop). Non-link
/// events (Fire/Alloc/Free/Sync) are skipped — Sync (`irq_barrier`) is a
/// no-op for these schedules (every data dep rides a blocking `link_recv`),
/// so it imposes no boot-order constraint of its own.
fn link_frame(events: &[Event]) -> Vec<LinkAction> {
    let mut out = Vec::new();
    fn walk(events: &[Event], out: &mut Vec<LinkAction>) {
        for e in events {
            match e {
                Event::Wait { seq, .. } => out.push(LinkAction::Recv { seq: seq.0 }),
                Event::Push { seq, dst, .. } => out.push(LinkAction::Push {
                    seq: seq.0,
                    dst: *dst,
                }),
                Event::Loop { body, .. } => walk(body, out),
                _ => {}
            }
        }
    }
    walk(events, &mut out);
    out
}

/// Compute the `.resc` machine release order so that **a worker's first
/// `link_push` to a peer P never executes before P has been released and run
/// `init()` (enabling RX)** — the binding start-gating invariant
/// (TASK-0049.01: Renode's `UARTBase.WriteChar` silently DROPS bytes that
/// arrive before the receiver enables RX).
///
/// The previous heuristic — sort by `(waits_before_first_push DESC, worker_id
/// ASC)` — does NOT encode that invariant: it got ex14 (TASK-0049.05.03)
/// wrong by luck. ex14's interconnect is CYCLIC (dsp↔fe, dsp↔rf), so there is
/// no universal static "receivers-first" order; but ex14 DOES have a valid
/// deterministic order (dsp, rf, fe). We find it by SIMULATING the staged
/// release as a small deterministic fixpoint instead of a one-shot sort:
///
///   * A worker that `Recv`s before its first `Push` (e.g. ex14 `dsp`) is
///     RECEIVE-GATED: releasing it early is harmless — it blocks on the
///     `link_recv` until its producer pushes, issuing no premature TX.
///   * Each round, greedily pick the smallest-`worker_id` not-yet-released
///     worker whose release (plus any transitively-unblocked pushes in
///     already-released receive-gated workers) issues NO `Push` targeting a
///     not-yet-released peer. That push would drop its bytes, so such a
///     worker is NOT yet safe to release.
///   * For ex14 this yields dsp, then rf, then fe deterministically.
///
/// FALLBACK / HONEST LIMIT: if a round finds NO safe worker (a genuine
/// mutual-eager-send cycle — two workers each whose first action is a Push to
/// the other), no static boot order is sound. We then append the remaining
/// workers in `worker_id` ASC order and the order MAY drop the opening bytes
/// of that mutual send. A robust fix for that case is a retransmit-until-acked
/// RX-ready handshake (the deferred TASK-0049.05.02 item-1 "cyclic RX-ready
/// handshake"); it is NOT implemented here. The tier-3 acceptance gate is the
/// byte-exact Renode diff, which fails LOUD on such a drop, so this cannot
/// ship a silent miscompile undetected — but it is an honest limit, not
/// universal robustness.
pub(super) fn compute_boot_order(
    per_worker: &BTreeMap<WorkerId, Vec<Event>>,
    used: &[WorkerId],
) -> Vec<WorkerId> {
    // Per-worker frame of link actions (Push/Recv), in program order.
    let frames: BTreeMap<WorkerId, Vec<LinkAction>> = used
        .iter()
        .map(|w| {
            let evs = per_worker.get(w).map(Vec::as_slice).unwrap_or(&[]);
            (*w, link_frame(evs))
        })
        .collect();

    let mut released: Vec<WorkerId> = Vec::with_capacity(used.len());
    let mut remaining: BTreeSet<WorkerId> = used.iter().copied().collect();

    while !remaining.is_empty() {
        // Try candidates in deterministic worker_id ASC order; pick the first
        // whose release keeps the no-premature-TX invariant (BTreeSet iterates
        // ascending).
        let pick = remaining.iter().copied().find(|&cand| {
            let mut trial: BTreeSet<WorkerId> = released.iter().copied().collect();
            trial.insert(cand);
            release_is_safe(&frames, &trial)
        });

        match pick {
            Some(w) => {
                released.push(w);
                remaining.remove(&w);
            }
            None => {
                // Genuine mutual-eager-send cycle: no static order is sound.
                // Append the rest deterministically; the byte-exact Renode
                // gate fails loud if the opening bytes drop. See the doc
                // comment's FALLBACK note + TASK-0049.05.02 item-1.
                released.extend(remaining.iter().copied());
                break;
            }
        }
    }

    released
}

/// Given the set of workers RELEASED so far (RX enabled), simulate every
/// released worker advancing through its `LinkAction` frame as far as it can,
/// and return `true` iff NO released worker issues a `Push` to a NOT-released
/// peer. A `Recv` blocks the worker until its producer (necessarily another
/// released worker — an unreleased worker has issued no push) has advanced
/// past the matching `Push`. Because every `Push` to an unreleased peer marks
/// the order unsafe, the order this drives is exactly "RX-before-TX holds for
/// every cross-MCU edge".
///
/// The fixpoint terminates: each round either advances at least one worker's
/// program counter (bounded by total link actions) or makes no progress (all
/// remaining recvs block on not-yet-pushed seqs) and stops.
fn release_is_safe(
    frames: &BTreeMap<WorkerId, Vec<LinkAction>>,
    released: &BTreeSet<WorkerId>,
) -> bool {
    // Program counter into each released worker's frame.
    let mut pc: BTreeMap<WorkerId, usize> = released.iter().map(|w| (*w, 0usize)).collect();
    // Seqs that have been PUSHED so far (so a matching Recv can unblock).
    let mut pushed: BTreeSet<u64> = BTreeSet::new();

    loop {
        let mut progressed = false;
        for w in released.iter() {
            let frame = match frames.get(w) {
                Some(f) => f,
                None => continue,
            };
            // Advance this worker over every action it can currently retire.
            loop {
                let i = pc[w];
                let Some(action) = frame.get(i) else { break };
                match action {
                    LinkAction::Recv { seq } => {
                        if pushed.contains(seq) {
                            *pc.get_mut(w).unwrap() = i + 1;
                            progressed = true;
                        } else {
                            // Blocked on an un-pushed producer; stop here.
                            break;
                        }
                    }
                    LinkAction::Push { seq, dst } => {
                        if !released.contains(dst) {
                            // Premature TX to an RX-disabled peer: UNSAFE.
                            return false;
                        }
                        pushed.insert(*seq);
                        *pc.get_mut(w).unwrap() = i + 1;
                        progressed = true;
                    }
                }
            }
        }
        if !progressed {
            break;
        }
    }
    true
}
