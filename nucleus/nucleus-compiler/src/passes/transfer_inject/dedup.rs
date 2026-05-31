use super::*;

/// True iff some earlier Xfer in `out` (within the current sync_inject
/// barrier epoch) matches `cand` on (role, src, dst, data, tile). Scans
/// from the end backward; stops at the first `ACFGNode::Sync` because a
/// barrier marks a fresh rendezvous epoch where a duplicate Wait is
/// legitimate (different consumer phase, different buffer place).
///
/// TASK-0335 cycle 158: introduced to dedupe Waits across multiple
/// consumer Operations in the same Sequence. The narrower
/// `is_duplicate_xfer(out.last(), ...)` only fires when the candidate
/// duplicates the immediately-preceding element; with an intervening
/// Operation it does not, and the duplicate Wait survives → duplicate
/// Pushes downstream → mp-tcp-bufsync runtime seq mismatch (the wire
/// FIFO inverts the producer-side splice order vs receiver Wait order).
/// The Sequence-scope scan suppresses the duplicate at source, so
/// `splice_pushes_global` emits one Push per surviving Wait's seq.
///
/// Invariant preserved: `inject_in_sequence(inject_in_sequence(x)) ==
/// inject_in_sequence(x)` — on re-run, every surviving Wait already
/// matches itself at index 0..N, and broader scan keeps suppressing.
/// The shape under re-run is the same shape produced by the first run.
///
/// TASK-0335.02 cycle 159: extended as the single source of truth for
/// "duplicate within an epoch in tail-anchored insertion shape" — the
/// `place_or_bubble` closure in `hoist_invariant_waits` previously
/// used an inline whole-`out` `out.iter().any(matches!(…))` scan
/// keyed on `(role, src, dst, data)` with NO Sync-stop. That site
/// APPENDS at the tail of `out`, so the tail-anchored backward scan
/// here is the structurally correct shape. The cycle-158 widening
/// pattern (4-tuple → 5-tuple) is what was applied. See
/// `is_duplicate_xfer_in_epoch_at_slot` for the SIBLING site
/// (`inject_in_sequence`'s hoisted-Waits-drain, TASK-0335.01) which
/// inserts at an arbitrary slot and therefore needed a different
/// helper rather than a parameter on this one — see that helper's
/// docstring for the new-fn-vs-param rationale.
///
/// **Choice rationale (widen vs assert), TASK-0335.02 only:** the
/// follow-up permitted either widening the helper's coverage or
/// asserting a structural invariant that the existing `out` is never
/// crossed by a Sync at the dedup point. We chose widen because
/// (a) failure mode of the invariant breaking is silent deadlock —
/// the worst kind; (b) widening is a strict superset of the pre-fix
/// correctness (no in-tree shape exercises the new arm today, so no
/// behaviour regresses); (c) keeps one helper as the single source
/// of truth for the "tail-anchored append" shape so future dedup
/// sites of that shape cannot subtly diverge again.
pub(super) fn is_duplicate_xfer_in_epoch(out: &[ACFGNode], cand: &XferPlaceholder) -> bool {
    for n in out.iter().rev() {
        match n {
            ACFGNode::Sync(_) => return false,
            ACFGNode::Xfer(existing) => {
                if existing.role == cand.role
                    && existing.src == cand.src
                    && existing.dst == cand.dst
                    && existing.data == cand.data
                    && existing.tile == cand.tile
                {
                    return true;
                }
            }
            // Operations, Repeats, Sequences are transparent to the
            // scan — they neither match nor terminate. (A Repeat's
            // body is its own walk context with its own `out` vec; if
            // a per-iteration Wait exists inside, it is not visible
            // here.)
            _ => {}
        }
    }
    false
}

/// True iff some existing Xfer in `out` matches `cand` on
/// (role, src, dst, data, tile) WITHIN THE EPOCH AROUND `slot` —
/// i.e. scanning backward from `slot - 1` until the first
/// `ACFGNode::Sync` (or start), AND forward from `slot` until the
/// first `ACFGNode::Sync` (or end).
///
/// This is the slot-aware sibling of [`is_duplicate_xfer_in_epoch`].
/// The tail-scoped variant assumes the candidate is being APPENDED
/// at the end of `out`, so a tail-anchored backward scan covers the
/// candidate's epoch. The slot-aware variant supports callers that
/// insert at an arbitrary slot (notably the hoisted-Waits-drain at
/// the tail of `inject_in_sequence` — `out.insert(slot, ...)`), where
/// the candidate's epoch may be entirely interior to `out` and
/// flanked by Syncs on either side.
///
/// TASK-0335.01 cycle 159: introduced after audit of cycle-158's
/// helper showed it was the right shape for the per-Op inline emit
/// site (always appends at tail) but the WRONG shape for the
/// hoisted-Waits-drain. The drain processes sibling block-inner
/// Repeats' waits in reverse-slot order; if two siblings flank a
/// Sync, the first-processed (later-slot) sibling's Wait gets
/// inserted post-Sync, and the second-processed (earlier-slot)
/// sibling's candidate then matched it under either pre-cycle-159's
/// whole-`out` scan OR cycle-158's tail-anchored backward scan,
/// causing silent over-suppression of the EARLIER-slot sibling. The
/// slot-aware bidirectional scan respects the Sync between them.
///
/// **Choice rationale (new helper vs `slot: Option<usize>` parameter
/// on `is_duplicate_xfer_in_epoch`):** kept as a separate function
/// because (a) the scan SHAPE is fundamentally different
/// (bidirectional from slot vs tail-anchored backward), not just a
/// scope refinement — folding both into one fn would require a
/// runtime branch on every call and obscure each shape's
/// invariants; (b) the per-call performance penalty of a unified
/// helper would be non-trivial (each call pays either always-both-
/// arms or branch-on-slot==len); (c) the two call sites are
/// structurally distinct (append-at-tail vs insert-at-slot), so the
/// separation matches the call-site shape rather than hiding it.
///
/// Idempotence is preserved (the primary purpose of the dedup): on
/// re-run, the previously-inserted Wait sits at exactly `slot`; the
/// forward scan starts at `slot` and finds it on the first
/// iteration → suppress (skip the re-emit). The Sync-stop in EITHER
/// direction does not interfere because the candidate sits between
/// the two Syncs flanking its own epoch, same as the first-run
/// insertion.
///
/// **Latent at cycle 159**: no in-tree schedule produces two
/// block-inner sibling Repeats separated by a Sync that share a
/// matching hoist-drain Wait key. Fixed defensively to defend
/// against future schedules.
pub(super) fn is_duplicate_xfer_in_epoch_at_slot(
    out: &[ACFGNode],
    slot: usize,
    cand: &XferPlaceholder,
) -> bool {
    let xfer_matches = |existing: &XferPlaceholder| -> bool {
        existing.role == cand.role
            && existing.src == cand.src
            && existing.dst == cand.dst
            && existing.data == cand.data
            && existing.tile == cand.tile
    };
    // Backward from slot-1 to start, stop at first Sync.
    if slot > 0 {
        for n in out[..slot].iter().rev() {
            match n {
                ACFGNode::Sync(_) => break,
                ACFGNode::Xfer(existing) if xfer_matches(existing) => return true,
                _ => {}
            }
        }
    }
    // Forward from slot to end, stop at first Sync. (The new Wait
    // would be inserted AT `slot`, so an existing Wait at `slot`
    // sits inside the same forward span as everything else up to the
    // next Sync.)
    for n in out.iter().skip(slot) {
        match n {
            ACFGNode::Sync(_) => break,
            ACFGNode::Xfer(existing) if xfer_matches(existing) => return true,
            _ => {}
        }
    }
    false
}
