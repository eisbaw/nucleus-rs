use super::*;

/// Walk a sequence of children, injecting Wait placeholders before
/// each Operation that reads cross-worker data and Push placeholders
/// after each Operation that produces data that someone downstream
/// will read on a different worker.
///
/// Implementation note: we emit *both* Wait (before the consumer) and
/// Push (after the producer) in a single linear scan. For each
/// consumer-side read of `D` whose producer entity differs from the
/// consumer's, we walk back through the children we've already
/// emitted to find the most recent producer of `D`, and splice a
/// Push placeholder immediately after it. That keeps the rewrite
/// local — no second pass needed.
pub(super) fn inject_in_sequence(
    children: Vec<ACFGNode>,
    ctx: &InjectCtx<'_>,
    state: &mut State,
    enclosing_tile: &[(IterVar, std::ops::Range<i64>)],
    // If non-None, this sequence sits *inside* a `block`-inner
    // intra-tile loop chain: any non-locally-produced Wait we
    // would emit gets diverted into the sink instead of staying
    // here, so the eventual destination sequence (the nearest
    // ancestor whose `parent_sink` was None) places the Wait at
    // per-tile granularity.
    mut parent_sink: Option<&mut HoistSink>,
) -> Vec<ACFGNode> {
    let mut out: Vec<ACFGNode> = Vec::with_capacity(children.len());

    // For each data symbol we've already produced in this sequence,
    // remember the index of the producing Operation in `out`. We use
    // this to splice a Push placeholder immediately after it.
    // BTreeMap for deterministic iteration; only the per-key insert
    // matters semantically.
    let mut local_producer_idx: BTreeMap<DataId, usize> = BTreeMap::new();

    // Buffer of (insert-before-index-in-out, Wait) for Xfers hoisted
    // out of an inner block-loop sibling that THIS sequence is the
    // destination for. Only populated when `parent_sink` is None at
    // the point we encounter a block-inner Repeat child. Drained into
    // `out` at the end of the walk.
    let mut hoisted_waits_to_place: Vec<(usize, XferPlaceholder)> = Vec::new();
    // Local sink (one per block-inner child we are draining for).
    // Held outside the loop so its lifetime spans the loop.
    let mut local_sink: HoistSink = HoistSink::default();
    let parent_sink_is_some = parent_sink.is_some();

    for child in children {
        // Peek at the child's shape before the recursion.
        let child_is_block_inner_repeat = matches!(
            &child,
            ACFGNode::Repeat { iter_var, .. }
                if ctx.inner_block_iter_vars.contains(iter_var)
        );

        let child = if child_is_block_inner_repeat && parent_sink_is_some {
            // We're not the destination; forward hoisted Waits
            // through to our parent's sink. `as_deref_mut` reborrows
            // for the duration of the recursion.
            let sink_ref: &mut HoistSink = parent_sink.as_deref_mut().expect("checked");
            inject_in_node_with_tile(child, ctx, state, enclosing_tile, Some(sink_ref))
        } else if child_is_block_inner_repeat {
            // We are the per-tile destination. Pass our `local_sink`
            // to the recursion; record the slot where the rewritten
            // Repeat will land so we can place hoisted Waits
            // immediately before it; then move the sink's contents
            // into `hoisted_waits_to_place` for the post-walk drain.
            let rewritten =
                inject_in_node_with_tile(child, ctx, state, enclosing_tile, Some(&mut local_sink));
            let slot = out.len();
            // Drain local_sink between block-inner siblings.
            for w in std::mem::take(&mut local_sink.waits) {
                hoisted_waits_to_place.push((slot, w));
            }
            rewritten
        } else if parent_sink_is_some {
            // Non-block-inner child inside an inner-block context:
            // propagate parent_sink so Waits emitted further down
            // can still hoist past this scope.
            let sink_ref: &mut HoistSink = parent_sink.as_deref_mut().expect("checked");
            inject_in_node_with_tile(child, ctx, state, enclosing_tile, Some(sink_ref))
        } else {
            // Plain non-block-inner child outside any inner-block
            // context: recurse with no sink. Standard pre-TASK-0143
            // path.
            inject_in_node_with_tile(child, ctx, state, enclosing_tile, None)
        };

        match &child {
            ACFGNode::Operation(op) => {
                // ---- Wait injection (consumer side) ----
                let waits = build_waits_for_op(op, ctx, state, enclosing_tile);
                for w in waits {
                    // Sequence-scope dedup: skip the Wait if an
                    // earlier Wait in this same Sequence (within the
                    // current sync_inject barrier epoch) already
                    // matches on (role, src, dst, data, tile). The
                    // scan stops at the first ACFGNode::Sync — a
                    // barrier marks a fresh rendezvous epoch where a
                    // duplicate Wait is legitimate (different consumer
                    // phase, different buffer place).
                    //
                    // TASK-0335 cycle 158: when two consumer Operations
                    // in the same Sequence read the same cross-worker
                    // data (e.g. host's two `combine` ops both reading
                    // `partials` in 03-reduction/distributed), the
                    // earlier dedup keyed on `out.last()` missed the
                    // collision (the intervening Operation between the
                    // two Wait-bursts pushed the first Wait out of
                    // last-position). The result was N duplicate Waits
                    // → splice_pushes_global splices N duplicate Pushes
                    // → mp-tcp-bufsync runtime seq-tag mismatch panic
                    // (wire FIFO ordering inverts vs receiver's Wait
                    // sequence). The dedup at this site is the source
                    // fix; once the duplicate Wait never enters the
                    // ACFG, splice_pushes_global naturally emits one
                    // Push per surviving Wait's seq.
                    if !is_duplicate_xfer_in_epoch(&out, &w) {
                        out.push(ACFGNode::Xfer(w));
                    }
                }

                // ---- Update local_producer_idx for any data this op
                //      writes. We record the index AFTER pushing the
                //      Operation so the Push insertion targets the
                //      right slot. ----
                let idx_after_push = out.len();
                update_writer(state, op);
                out.push(child.clone());
                if let Some(data_out) = output_data(op) {
                    local_producer_idx.insert(data_out, idx_after_push);
                }

                // ---- Push injection (producer side) for any data
                //      this op produces whose downstream consumer
                //      will be cross-worker. We can't know future
                //      consumers up-front, so we add Pushes lazily
                //      when a future consumer is seen (see the
                //      Wait-handling branch above): the moment a
                //      Wait is emitted for (src, dst, D, tile), we
                //      also splice a Push at `local_producer_idx[D]
                //      + 1` if not already present. That happens in
                //      the next iteration; see below. ----
            }
            _ => {
                // Non-Operation nodes (Repeat, Sequence, Sync, Xfer):
                // already rewritten by recursion. Push as-is.
                out.push(child);
            }
        }
    }

    // Drain hoisted Waits: insert each (slot, Wait) into `out` so
    // the Wait sits immediately before the inner Repeat. Inserting
    // shifts later indices, so we process by descending slot. For a
    // given slot, multiple Waits keep their relative order — same
    // semantics as the in-line emission path.
    //
    // Before placement, rewrite each hoisted Wait's `IterTile` to
    // match THIS sequence's enclosing tile. The consumer's Wait was
    // built deeper in the nest and carries axes for every loop
    // between the consumer and the original block-inner; once
    // hoisted to the per-tile destination, only the axes enclosing
    // this destination should remain. Replacing the tile wholesale
    // is simpler than tracking which axes were crossed during the
    // hoist forward, and matches the PRD §6.3.3 "transfers happen
    // per tile" semantic: the Wait fires at this sequence's
    // enclosing-tile granularity.
    for (_, w) in hoisted_waits_to_place.iter_mut() {
        w.tile = IterTile::new(enclosing_tile.to_vec());
    }
    if !hoisted_waits_to_place.is_empty() {
        // Stable sort by slot ascending so later (larger-slot)
        // inserts can be applied first without invalidating earlier
        // slots' positions.
        hoisted_waits_to_place.sort_by_key(|(s, _)| *s);
        // Group by slot, then for each group insert in reverse slot
        // order. Within a group, preserve original order by
        // inserting in reverse.
        let mut by_slot: BTreeMap<usize, Vec<XferPlaceholder>> = BTreeMap::new();
        for (s, w) in hoisted_waits_to_place {
            by_slot.entry(s).or_default().push(w);
        }
        // Iterate in *reverse* slot order. Iter rev on BTreeMap.
        let mut shifts: Vec<(usize, Vec<XferPlaceholder>)> = by_slot.into_iter().collect();
        shifts.sort_by_key(|(s, _)| *s);
        for (slot, waits) in shifts.into_iter().rev() {
            // Insert each Wait at `slot`, in reverse so the first
            // listed Wait ends up first in `out`.
            for w in waits.into_iter().rev() {
                // Idempotence: skip if `out` already contains an
                // identical Wait WITHIN THE EPOCH AROUND `slot`
                // (scan backward AND forward from `slot`, both
                // stopping at the first `ACFGNode::Sync`). The
                // bidirectional scan is required because this site
                // inserts AT `slot` (typically far from the tail of
                // `out`), so the tail-scoped cycle-158 helper would
                // find a sibling-drain's Wait on the WRONG side of
                // an intervening Sync and silently over-suppress.
                //
                // TASK-0335.01 cycle 159: introduced
                // `is_duplicate_xfer_in_epoch_at_slot` to address the
                // LATENT cross-Sync drain bug (two block-inner
                // sibling Repeats separated by a Sync each drain the
                // same Wait into `hoisted_waits_to_place`; the
                // pre-cycle-159 whole-`out` scan suppressed one of
                // them; the cycle-158 tail-scan helper would have
                // had the same defect, just choosing a different
                // sibling to drop). A barrier marks a fresh
                // rendezvous epoch where a hoisted-drain Wait is
                // legitimate (different consumer phase, different
                // buffer place); the slot-aware helper preserves
                // both. Idempotence is preserved because re-running
                // re-derives the Wait at the SAME slot; the forward
                // scan finds the prior-run Wait at that slot before
                // any Sync → suppress (skip the re-emit). The
                // candidate's role is always Wait here, which the
                // helper checks.
                if is_duplicate_xfer_in_epoch_at_slot(&out, slot, &w) {
                    continue;
                }
                out.insert(slot, ACFGNode::Xfer(w));
            }
        }
        // Recompute `local_producer_idx` -- insertions shifted some
        // recorded indices. Cheap to walk `out` once.
        local_producer_idx.clear();
        for (i, n) in out.iter().enumerate() {
            if let ACFGNode::Operation(op) = n {
                if let Some(data_out) = output_data(op) {
                    local_producer_idx.insert(data_out, i);
                }
            }
        }
    }

    // Second pass: for every Wait we emitted (in-line *or* hoisted)
    // in this sequence, walk back and splice a matching Push
    // immediately after the producer Operation (which is in `out`
    // because the producer's Operation appeared earlier in the same
    // sequence) — *unless* the producer is in a different scope
    // (outer sequence, sibling sequence). In that case the
    // outer-scope walker handles it. We detect "different scope" by
    // checking `local_producer_idx`: if the Wait names a data symbol
    // not in `local_producer_idx`, the producer is not in this
    // sequence and we leave the Push to the caller.
    splice_pushes_for_waits(&mut out, &local_producer_idx);

    // If this sequence is *itself* sitting inside a block-inner
    // Repeat chain (i.e. our caller passed a `parent_sink`), forward
    // any Waits we just placed up to the parent so they land at the
    // eventual destination's per-tile granularity. We only forward
    // Waits whose producer was NOT in this sequence — in-sequence
    // Push/Wait pairs already form a complete intra-tile rendezvous
    // and need not propagate further.
    //
    // Tile rewrite happens at the destination (see the drain block
    // above); here we simply hand the Wait off unchanged.
    if let Some(sink) = parent_sink {
        let mut kept: Vec<ACFGNode> = Vec::with_capacity(out.len());
        for node in out.into_iter() {
            match node {
                ACFGNode::Xfer(x)
                    if x.role == XferRole::Wait && !local_producer_idx.contains_key(&x.data) =>
                {
                    sink.waits.push(x);
                }
                other => kept.push(other),
            }
        }
        out = kept;
    }

    out
}

/// Walk `out` left-to-right; for every Wait placeholder, look up the
/// producer Operation in `local_producer_idx` and splice a Push
/// placeholder right after it (with the same `seq` as the Wait).
///
/// Splicing later items shifts indices, so we collect insertion
/// requests first and then apply them in reverse order to keep the
/// recorded indices valid.
pub(super) fn splice_pushes_for_waits(
    out: &mut Vec<ACFGNode>,
    local_producer_idx: &BTreeMap<DataId, usize>,
) {
    // (insert_at, placeholder)
    let mut inserts: Vec<(usize, XferPlaceholder)> = Vec::new();

    for n in out.iter() {
        if let ACFGNode::Xfer(x) = n {
            if x.role == XferRole::Wait {
                if let Some(&producer_idx) = local_producer_idx.get(&x.data) {
                    // Push goes immediately after the producer
                    // Operation, i.e. at producer_idx + 1.
                    let insert_at = producer_idx + 1;
                    let push = XferPlaceholder {
                        role: XferRole::Push,
                        src: x.src,
                        dst: x.dst,
                        data: x.data,
                        tile: x.tile.clone(),
                        seq: x.seq,
                        policy: x.policy,
                    };
                    inserts.push((insert_at, push));
                }
            }
        }
    }

    // Apply in reverse index order so earlier insertions don't
    // invalidate the later ones. The reverse-apply order is order-safe
    // for co-located Pushes ONLY because v2 is single-assignment: each
    // datum has exactly one producer Operation, so distinct data get
    // distinct `insert_at` indices and never co-locate at one slot (the
    // same invariant that makes `splice_after_producer`'s append a
    // no-op). If a future multi-output Operation is introduced, this
    // site and `splice_after_repeat`/`splice_after_producer` must be
    // revisited TOGETHER for per-channel Push ordering (TASK-0389.01).
    inserts.sort_by_key(|(i, _)| *i);
    for (insert_at, push) in inserts.into_iter().rev() {
        // Idempotence: if the slot immediately after the producer is
        // already a Push with matching (src, dst, data, tile), skip.
        let already = out.get(insert_at).and_then(|n| match n {
            ACFGNode::Xfer(x) => Some(x.clone()),
            _ => None,
        });
        if let Some(existing) = already {
            if existing.role == XferRole::Push
                && existing.src == push.src
                && existing.dst == push.dst
                && existing.data == push.data
                && existing.tile == push.tile
            {
                continue;
            }
        }
        out.insert(insert_at, ACFGNode::Xfer(push));
    }
}

// --------------------------------------------------------------------
// Pass A — hoist loop-invariant Waits out of plain Repeat bodies
// --------------------------------------------------------------------
//
// TASK-0136 / TASK-0139. `inject_in_sequence` emits a Wait immediately
// before the consumer Operation. When the consumer sits inside a
// `for` loop but the data it reads is produced *outside* the loop
// (loop-invariant by structure: the data symbol is not written by any
// Operation inside the loop body), the Wait should fire once before
// the loop, not once per iteration. The Petri lowering unrolls Repeat
// bodies (`acfg_to_petri::walk`), so a per-iteration Wait against a
// single whole-symbol Push deadlocks at iteration 2 (one token, N
// consumers). Hoisting the Wait out of the body fixes this and matches
// what a real backend does (transfer the whole symbol once).
//
// Returns the rewritten node plus the Waits that escaped past the top
// of `node` and must be placed (or further hoisted) by the caller.

/// Collect every data symbol written by some Operation anywhere in the
/// subtree rooted at `node`.
pub(super) fn produced_data_set(node: &ACFGNode, acc: &mut BTreeSet<DataId>) {
    match node {
        ACFGNode::Operation(op) => {
            if let Some(d) = output_data(op) {
                acc.insert(d);
            }
        }
        ACFGNode::Sync(_) | ACFGNode::Xfer(_) => {}
        ACFGNode::Repeat { body, .. } => produced_data_set(body, acc),
        ACFGNode::Sequence(children) => {
            for c in children {
                produced_data_set(c, acc);
            }
        }
    }
}

/// Number of Operations in the subtree that write `data`. Used only
/// by a debug-assert guarding the single-assignment invariant
/// (TASK-0153); not on the release hot path.
pub(super) fn count_producers(node: &ACFGNode, data: DataId) -> usize {
    match node {
        ACFGNode::Operation(op) => usize::from(output_data(op) == Some(data)),
        ACFGNode::Sync(_) | ACFGNode::Xfer(_) => 0,
        ACFGNode::Repeat { body, .. } => count_producers(body, data),
        ACFGNode::Sequence(children) => children.iter().map(|c| count_producers(c, data)).sum(),
    }
}

pub(super) fn hoist_invariant_waits(
    node: ACFGNode,
    enclosing_tile: &[(IterVar, std::ops::Range<i64>)],
) -> (ACFGNode, Vec<XferPlaceholder>) {
    match node {
        leaf @ (ACFGNode::Operation(_) | ACFGNode::Sync(_)) => (leaf, Vec::new()),
        // A bare Wait/Xfer outside a Sequence shouldn't occur from the
        // builder, but if it does we cannot decide invariance without
        // its sibling context — leave it in place.
        leaf @ ACFGNode::Xfer(_) => (leaf, Vec::new()),
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            block_tag,
            break_cond,
        } => {
            let mut nested: Vec<(IterVar, std::ops::Range<i64>)> = enclosing_tile.to_vec();
            nested.push((iter_var, range.clone()));
            let (body2, escaped) = hoist_invariant_waits(*body, &nested);

            let mut produced = BTreeSet::new();
            produced_data_set(&body2, &mut produced);

            // A Wait that escaped the body but whose data IS produced
            // inside this loop is NOT loop-invariant w.r.t. this
            // Repeat: it belongs inside, as a per-iteration rendezvous.
            // Re-inject it at the front of the body. Otherwise it keeps
            // bubbling up.
            let mut stay: Vec<XferPlaceholder> = Vec::new();
            let mut bubble: Vec<XferPlaceholder> = Vec::new();
            for w in escaped {
                if produced.contains(&w.data) {
                    stay.push(w);
                } else {
                    bubble.push(w);
                }
            }

            let body3 = if stay.is_empty() {
                body2
            } else {
                let mut children: Vec<ACFGNode> = stay.into_iter().map(ACFGNode::Xfer).collect();
                match body2 {
                    ACFGNode::Sequence(cs) => children.extend(cs),
                    other => children.push(other),
                }
                ACFGNode::Sequence(children)
            };

            (
                ACFGNode::Repeat {
                    iter_var,
                    range,
                    body: Box::new(body3),
                    // Wait-hoisting only reshuffles the body; the
                    // strip-mine rebinding tag is preserved verbatim
                    // (TASK-0180).
                    block_tag,
                    // ... likewise the `for..until` halt predicate is
                    // carried through unchanged.
                    break_cond,
                },
                bubble,
            )
        }
        ACFGNode::Sequence(children) => {
            // Recurse non-Wait children; hold direct Wait Xfers aside
            // so we can decide per-Wait whether it stays here (its data
            // is produced in this sequence: an intra-scope rendezvous)
            // or bubbles up (loop-invariant, escape the enclosing
            // Repeat).
            enum Slot {
                Node(ACFGNode),
                Wait(XferPlaceholder),
            }
            let mut slots: Vec<(Slot, Vec<XferPlaceholder>)> = Vec::new();
            for child in children {
                match child {
                    ACFGNode::Xfer(x) if x.role == XferRole::Wait => {
                        slots.push((Slot::Wait(x), Vec::new()));
                    }
                    other => {
                        let (c2, esc) = hoist_invariant_waits(other, enclosing_tile);
                        slots.push((Slot::Node(c2), esc));
                    }
                }
            }

            // Data produced anywhere in this sequence (post-recursion).
            let mut produced = BTreeSet::new();
            for (slot, _) in &slots {
                if let Slot::Node(n) = slot {
                    produced_data_set(n, &mut produced);
                }
            }

            let mut out: Vec<ACFGNode> = Vec::new();
            let mut escaped_up: Vec<XferPlaceholder> = Vec::new();

            let place_or_bubble =
                |w: XferPlaceholder,
                 out: &mut Vec<ACFGNode>,
                 escaped_up: &mut Vec<XferPlaceholder>| {
                    if produced.contains(&w.data) {
                        // Lands here: rewrite tile to this sequence's
                        // enclosing-tile granularity and dedup against
                        // an already-placed equivalent Wait WITHIN the
                        // same sync_inject barrier epoch (scan stops at
                        // ACFGNode::Sync — a barrier marks a fresh
                        // rendezvous epoch where a fresh hoist target
                        // is legitimate). Keeps the pass idempotent on
                        // re-run: the regenerated Wait carries a fresh
                        // seq, but the structural (role,src,dst,data,
                        // tile) key is stable, so we keep the first
                        // and drop the duplicate within the epoch.
                        //
                        // TASK-0335.02 cycle 159: routed through the
                        // same Sync-stopping helper as cycle-158's
                        // inline emit-site. The pre-cycle-159 form was
                        // a whole-`out` scan keyed on (role,src,dst,
                        // data) with NO Sync-stop — strictly more
                        // aggressive than the cycle-158 fix. Without
                        // the Sync-stop, a legitimate hoist target on
                        // the FAR side of a barrier would be silently
                        // suppressed by a matching earlier-epoch Wait
                        // → deadlock (different buffer places).
                        //
                        // The `tile` component of the helper's key is
                        // SAFE here (cycle-159 architect P2.1
                        // correction; supersedes the cycle-159 initial
                        // "every Wait was placed by THIS closure"
                        // claim, which was false — `Slot::Wait` push
                        // (the `out.push(ACFGNode::Xfer(x))` arm in
                        // the slot loop further down) ALSO places
                        // Waits without routing through this closure
                        // and without rewriting tile). Why it's still
                        // safe: a Slot::Wait-pushed Wait carries the
                        // tile from upstream inject_in_node_with_tile
                        // depth-tracking (Repeat→Sequence descent
                        // appends to enclosing_tile, just as
                        // hoist_invariant_waits's own Repeat handler
                        // appends to its `nested` accumulator); at the
                        // same Sequence depth, the two trackers
                        // produce structurally identical tiles, so a
                        // candidate rewritten to enclosing_tile
                        // matches a Slot::Wait Wait at the SAME depth
                        // by chance — and matches a place_or_bubble-
                        // pushed Wait by construction. A
                        // Slot::Wait-pushed Wait at a DIFFERENT depth
                        // would not match the 5-tuple key, which is
                        // the SAFE direction (less-aggressive dedup,
                        // not silent over-suppression). Keeping the
                        // 5-tuple helper costs one extra comparison
                        // per element but keeps one single source of
                        // truth for "duplicate within an epoch".
                        let mut w = w;
                        w.tile = IterTile::new(enclosing_tile.to_vec());
                        if !is_duplicate_xfer_in_epoch(out, &w) {
                            out.push(ACFGNode::Xfer(w));
                        }
                    } else {
                        escaped_up.push(w);
                    }
                };

            for (slot, esc) in slots {
                // Escaped Waits from this child (a Repeat) are placed
                // immediately *before* the child.
                for w in esc {
                    place_or_bubble(w, &mut out, &mut escaped_up);
                }
                match slot {
                    Slot::Node(n) => out.push(n),
                    Slot::Wait(x) => {
                        if produced.contains(&x.data) {
                            // Intra-scope rendezvous: producer is a
                            // sibling here. Keep the Wait where it is.
                            out.push(ACFGNode::Xfer(x));
                        } else {
                            // Loop-invariant w.r.t. the Repeat that
                            // encloses this sequence: hoist it out.
                            escaped_up.push(x);
                        }
                    }
                }
            }

            (ACFGNode::Sequence(out), escaped_up)
        }
    }
}
