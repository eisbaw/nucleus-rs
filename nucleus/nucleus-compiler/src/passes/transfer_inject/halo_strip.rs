use super::*;

// --------------------------------------------------------------------
// TASK-0289 cycle 114a halo-strip Push/Wait synthesis
// --------------------------------------------------------------------

/// Synthesise cross-worker halo-strip Push/Wait pairs between N/S/E/W
/// neighbour cells in a 2D worker grid established by
/// `partition=blocks2d` (TASK-0289 AC#1).
///
/// ### Why this lives at the tail of `inject_transfers`
///
/// `rewrite_partition_tiles` walks every `Xfer` and unconditionally
/// rewrites the tile from the partition sidecar (compute-worker rule);
/// `extend_xfer_tiles_for_halo` walks every `Xfer` and extends each tile
/// axis by the consumer kernel's halo width. Both rewrite in-place. A
/// synthesised halo-strip transfer carries a precisely-bounded
/// `[lo .. hi)` per axis (the strip-side of the neighbour's band, NOT
/// the worker's own partition slice + halo); if it ran BEFORE those
/// passes, the partition rewrite would replace its tile with the
/// src-worker's full band and the halo extension would re-pad it from
/// the wrong base. Running AFTER lets the strip-tile survive verbatim.
///
/// ### Why we synthesise BOTH Push and Wait pre-paired (instead of
/// only the Wait and letting `splice_pushes_global` produce the Push)
///
/// `splice_pushes_global` finds the matching Push location by walking
/// for the data symbol's PRODUCER Operation in the ACFG — but for a
/// halo strip on `img_in` (host-loaded), the producer is the host's
/// `load_image`, not the neighbour worker. Letting splice infer the
/// Push location would mis-route the Push to the host's scope; the
/// per-worker projection then emits a Push on `host`, not on the
/// neighbour worker. Synthesising both endpoints with a shared
/// `SeqTag` BEFORE splice runs would also be wrong (splice already
/// ran above by this point in the chain). So we run here, post-splice,
/// and create the pair with a `state.fresh_seq()` SeqTag pulled from
/// the shared monotonic counter — guaranteeing no SeqTag collision
/// with any pair already in the tree.
///
/// ### Per-Wait routing
///
/// The per-worker projection (`crate::passes::petri_to_events::emit_xfer`)
/// emits a Push event into `x.src`'s EventList and a Wait event into
/// `x.dst`'s EventList. So placing both Push and Wait nodes in the
/// SAME ACFG `Sequence` is fine: each worker's EventList picks up only
/// the endpoint matching its WorkerId. We insert the pairs as
/// siblings into whatever `Sequence` contains the outer Repeat for
/// `outer_iv`, AFTER the last producing Operation in that Sequence
/// (TASK-0290 cycle 114b — see `prepend_strip_pairs` for the
/// placement rule and fallback).
///
/// ### Strip tile math
///
/// For a worker at grid `(row, col)` with y-band `[y_lo, y_hi)`,
/// x-band `[x_lo, x_hi)`, and per-axis halo `h_y` / `h_x`:
///
/// - **N-strip** (received FROM `(row-1, col)`):
///   y in `[y_lo - h_y, y_lo)`, x in `[x_lo, x_hi)`
/// - **S-strip** (received FROM `(row+1, col)`):
///   y in `[y_hi, y_hi + h_y)`, x in `[x_lo, x_hi)`
/// - **W-strip** (received FROM `(row, col-1)`):
///   y in `[y_lo, y_hi)`, x in `[x_lo - h_x, x_lo)`
/// - **E-strip** (received FROM `(row, col+1)`):
///   y in `[y_lo, y_hi)`, x in `[x_hi, x_hi + h_x)`
///
/// Workers on an edge of the grid skip the off-grid neighbour.
/// Corners (NE/NW/SE/SW) are excluded in this first cut per the
/// TASK-0289 brief — the diagonal halo cell would require a separate
/// (worker, neighbour-diagonal) pair to be sound; filed as a follow-up.
///
/// ### Honest limitations (this cycle)
///
/// - **Placement**: pairs are PREPENDED to the parent Sequence that
///   contains the outer Repeat. For a single-pass stencil (no time
///   loop) this lands them at top-level. For a multi-pass / time-step
///   stencil, a future task should refine placement to "inside the
///   timestep Repeat, before the partitioned outer Repeat" so the
///   halo exchange fires per timestep. The current placement is
///   correct for the synthetic unit test and not yet exercised by
///   any shipped schedule (AC#3 short-circuit holds).
///
/// - **Not idempotent on re-run** when partition_pairs is non-empty.
///   On a re-run of `inject_transfers`, (1)
///   `rewrite_partition_tiles` clobbers strip tiles before this
///   synthesis sees them (its compute-worker rule replaces strip
///   with src's full partition slice) and (2)
///   `splice_pushes_for_waits` (inside `inject_in_sequence`)
///   splices a NEW Push for every halo-strip Wait it sees, because
///   the existing Pushes sit BEFORE the producer load Op (outside
///   its immediate-successor dedupe window). No production driver
///   path re-runs `inject_transfers`; the shipped-schedule
///   idempotence test (`tests/transfer_inject.rs::
///   idempotent_on_synthetic_two_worker_case`) stays green because
///   partition_pairs is empty for every shipped schedule and the
///   AC#3 guard short-circuits this entire function. Filed
///   forward-carried on TASK-0290.
///
/// - **No deduplication against existing halo Pushes**: if a later
///   pass adds halo pairs from a different join, two pairs for the
///   same `(src, dst, data, tile)` could co-exist. Today no such
///   later pass exists; the unit test asserts the exact pair count.
///
/// - **Policy**: each synthesised pair inherits the policy of its
///   data symbol from `policies_by_data` (the schedule's
///   `transfer D : ...` directive), falling back to
///   `TransferPolicy::default()` (sync, buffer=1, notify-default) when
///   the symbol has no directive. Matches the convention used by
///   `build_waits_for_op` for regular Push/Wait pairs.
///
/// ### Determinism
///
/// All iteration is over `BTreeMap` / `BTreeSet` (numeric order). The
/// per-axis emit order is fixed (N, S, W, E). SeqTags come from the
/// shared `state` counter in DFS-deterministic visit order. Same
/// input ⇒ byte-identical output.
#[allow(clippy::too_many_arguments)]
pub(super) fn inject_halo_strip_xfers(
    node: ACFGNode,
    halo_widths: &BTreeMap<KernelId, BTreeMap<IterVar, u64>>,
    partition_pairs: &BTreeMap<IterVar, IterVar>,
    grid_shape_for_outer_iv: &BTreeMap<IterVar, (u32, u32)>,
    partition_worker_ranges: &BTreeMap<IterVar, BTreeMap<WorkerId, std::ops::Range<i64>>>,
    policies_by_data: &BTreeMap<DataId, TransferPolicy>,
    data_dim_iv_map: &BTreeMap<DataId, Vec<BTreeSet<IterVar>>>,
    state: &mut State,
) -> ACFGNode {
    // AC#3 additive-only short-circuit. Every shipped schedule has an
    // empty `partition_pairs` (verified by the
    // `shipped_examples_without_blocks2d_leave_maps_empty` test in
    // `tests/sidecar_partition_blocks2d.rs`); the early return is the
    // structural guarantee that no existing Xfer count or shape
    // changes today.
    if partition_pairs.is_empty() {
        return node;
    }

    // Re-run dedupe is INTENTIONALLY OMITTED today. Per the comment
    // block in `tests/halo_strip_synth.rs` (search for IDEMPOTENCE /
    // RE-RUN CAVEAT), `inject_transfers` is not idempotent when
    // partition_pairs is non-empty: `rewrite_partition_tiles` clobbers
    // halo-strip tiles on a re-run, and `splice_pushes_for_waits`
    // splices a new Push for every halo-strip Wait it sees outside the
    // immediate-producer-neighbour dedupe window. Adding a 4-tuple
    // dedupe in this function alone would not restore structural
    // idempotence (the tile would still be wrong, and the new pushes
    // from splice_pushes_for_waits would still land). The full fix is
    // forward-carried to TASK-0290.
    //
    // No production code path re-runs `inject_transfers` on its own
    // output (the driver pipeline calls it once), so this is a
    // first-cut concession rather than a behavioural regression for
    // any shipped schedule.

    // Build (DataId -> Set<KernelId>) consumer index — same shape +
    // walk as `extend_xfer_tiles_for_halo`'s `collect_data_consumers`.
    // We need this to translate `halo_widths[kid][iv]` (keyed by
    // kernel id) into "which data symbols cross the halo boundary for
    // this consumer kernel".
    let mut consumers: BTreeMap<DataId, BTreeSet<KernelId>> = BTreeMap::new();
    collect_data_consumers(&node, &mut consumers);

    // For each (outer_iv, inner_iv) pair, build the list of
    // (data, src_worker, dst_worker, tile) halo-strip transfers to
    // synthesise. Group by outer_iv: each group is placed in the
    // parent Sequence containing the outer Repeat for that outer_iv
    // (TASK-0290 cycle 114b: placement is AFTER the last producing
    // Operation, not at the front — see `prepend_strip_pairs`).
    //
    // `BTreeMap<IterVar, Vec<XferPlaceholder>>` so the insertion order
    // matches outer_iv numeric order — deterministic.
    let mut to_insert: BTreeMap<IterVar, Vec<XferPlaceholder>> = BTreeMap::new();

    for (&outer_iv, &inner_iv) in partition_pairs {
        // Look up the grid shape recorded by partition_blocks2d.
        let Some(&(grid_rows, grid_cols)) = grid_shape_for_outer_iv.get(&outer_iv) else {
            // partition_pairs entry without a matching grid_shape entry
            // would be a partition_blocks2d invariant violation. Skip
            // defensively: the AC#3 short-circuit covers the "no pairs"
            // case; this is the belt-and-braces guard for malformed
            // sidecars on the per-pair path.
            continue;
        };
        let grid_cols_usize = grid_cols as usize;
        let grid_rows_usize = grid_rows as usize;

        // Per-worker y-band (from outer_iv) + x-band (from inner_iv).
        let Some(per_worker_y) = partition_worker_ranges.get(&outer_iv) else {
            continue;
        };
        let Some(per_worker_x) = partition_worker_ranges.get(&inner_iv) else {
            continue;
        };
        // Workers in row-major order — matches partition_blocks2d's
        // assignment (`for (i, wid) in body_workers.iter().enumerate()
        // { row = i / cols; col = i % cols }`). BTreeMap key iteration
        // is numeric WorkerId order, equivalent to the BTreeSet
        // iteration partition_blocks2d used on the write side.
        let body_workers: Vec<WorkerId> = per_worker_y.keys().copied().collect();
        if body_workers.len() != grid_rows_usize * grid_cols_usize {
            // Defensive: the row/col inversion only makes sense when
            // body_workers.len() == grid_rows * grid_cols. A mismatch
            // is a partition_blocks2d invariant violation; skip rather
            // than synthesise wrong pairs.
            continue;
        }

        // For each consumer kernel with a halo entry on outer_iv or
        // inner_iv, find every data symbol that kernel consumes. That
        // is the set of data symbols needing a strip transfer.
        //
        // `data_with_halo_y[data] = h_y` is the y-axis halo for the
        // FIRST consumer kernel matching that data (single-consumer
        // case in every in-tree example; documented as a limit for
        // multi-consumer cases — same caveat the per-tile halo
        // extension carries).
        let mut data_with_halo_y: BTreeMap<DataId, u64> = BTreeMap::new();
        let mut data_with_halo_x: BTreeMap<DataId, u64> = BTreeMap::new();
        for (data, kids) in &consumers {
            for kid in kids {
                if let Some(per_iv) = halo_widths.get(kid) {
                    if let Some(&h) = per_iv.get(&outer_iv) {
                        if h > 0 {
                            // Take MAX across consumer kernels — same
                            // merge rule `extend_xfer_tiles_for_halo`
                            // uses. The map's `entry().and_modify().or_insert`
                            // gives "max across kids" naturally.
                            data_with_halo_y
                                .entry(*data)
                                .and_modify(|cur| *cur = (*cur).max(h))
                                .or_insert(h);
                        }
                    }
                    if let Some(&h) = per_iv.get(&inner_iv) {
                        if h > 0 {
                            data_with_halo_x
                                .entry(*data)
                                .and_modify(|cur| *cur = (*cur).max(h))
                                .or_insert(h);
                        }
                    }
                }
            }
        }

        // Union of data symbols touched on either axis. Iteration in
        // BTreeSet numeric order — deterministic.
        let mut all_halo_data: BTreeSet<DataId> = BTreeSet::new();
        all_halo_data.extend(data_with_halo_y.keys().copied());
        all_halo_data.extend(data_with_halo_x.keys().copied());

        // No data symbols with halo on either partitioned axis ⇒
        // nothing to synthesise for this pair. Skip without inserting
        // an empty group (`to_insert.entry(outer_iv).or_default()`
        // would needlessly noise the placement walk).
        if all_halo_data.is_empty() {
            continue;
        }

        let group = to_insert.entry(outer_iv).or_default();

        // For each worker, compute (row, col), enumerate cardinal
        // neighbours, synthesise one Push+Wait pair per (data,
        // direction).
        //
        // Direction emit order is FIXED as (N, S, W, E) — the exact
        // order is a stability contract for the unit test's pair-
        // counting predicate. SeqTags monotonically increase in that
        // visit order, matching the per-worker BTreeMap iteration.
        for (i, &this_w) in body_workers.iter().enumerate() {
            let row = i / grid_cols_usize;
            let col = i % grid_cols_usize;
            let Some(y_band) = per_worker_y.get(&this_w) else {
                continue;
            };
            let Some(x_band) = per_worker_x.get(&this_w) else {
                continue;
            };
            let (y_lo, y_hi) = (y_band.start, y_band.end);
            let (x_lo, x_hi) = (x_band.start, x_band.end);

            // Helper: build the XferPlaceholder pair for a single
            // (src_worker, strip-tile, data) and push both Push and
            // Wait into `group`. No re-run dedupe today — see the
            // re-run-caveat comment near the top of this function.
            let mut emit_pair =
                |src: WorkerId, dst: WorkerId, data: DataId, tile: IterTile, state: &mut State| {
                    let policy = policies_by_data.get(&data).copied().unwrap_or_default();
                    let seq = state.fresh_seq();
                    group.push(XferPlaceholder {
                        role: XferRole::Push,
                        src,
                        dst,
                        data,
                        tile: tile.clone(),
                        seq,
                        policy,
                    });
                    group.push(XferPlaceholder {
                        role: XferRole::Wait,
                        src,
                        dst,
                        data,
                        tile,
                        seq,
                        policy,
                    });
                };

            // ---- N neighbour: row-1, col. ----
            //
            // Per-axis bands: outer (y) extends DOWN by h_y into the
            // neighbour's bottom row; inner (x) stays at this worker's
            // x-band. `order_halo_strip_bounds_by_data_dim` (TASK-0306)
            // orders the tile by data-dim position — preserving the
            // pre-cycle-133 `(outer, inner)` shape for canonical
            // outer-axis-leading layouts and flipping / dropping for
            // inner-axis-leading or non-prefix layouts.
            if row > 0 {
                let neighbour_idx = (row - 1) * grid_cols_usize + col;
                let neighbour = body_workers[neighbour_idx];
                for &data in &all_halo_data {
                    if let Some(&h_y) = data_with_halo_y.get(&data) {
                        let h_y_i: i64 = h_y.try_into().unwrap_or(i64::MAX);
                        let bounds = order_halo_strip_bounds_by_data_dim(
                            data,
                            outer_iv,
                            (y_lo - h_y_i)..y_lo,
                            inner_iv,
                            x_lo..x_hi,
                            data_dim_iv_map,
                        );
                        let tile = IterTile::new(bounds);
                        emit_pair(neighbour, this_w, data, tile, state);
                    }
                }
            }

            // ---- S neighbour: row+1, col. ----
            if row + 1 < grid_rows_usize {
                let neighbour_idx = (row + 1) * grid_cols_usize + col;
                let neighbour = body_workers[neighbour_idx];
                for &data in &all_halo_data {
                    if let Some(&h_y) = data_with_halo_y.get(&data) {
                        let h_y_i: i64 = h_y.try_into().unwrap_or(i64::MAX);
                        let bounds = order_halo_strip_bounds_by_data_dim(
                            data,
                            outer_iv,
                            y_hi..(y_hi + h_y_i),
                            inner_iv,
                            x_lo..x_hi,
                            data_dim_iv_map,
                        );
                        let tile = IterTile::new(bounds);
                        emit_pair(neighbour, this_w, data, tile, state);
                    }
                }
            }

            // ---- W neighbour: row, col-1. ----
            if col > 0 {
                let neighbour_idx = row * grid_cols_usize + (col - 1);
                let neighbour = body_workers[neighbour_idx];
                for &data in &all_halo_data {
                    if let Some(&h_x) = data_with_halo_x.get(&data) {
                        let h_x_i: i64 = h_x.try_into().unwrap_or(i64::MAX);
                        let bounds = order_halo_strip_bounds_by_data_dim(
                            data,
                            outer_iv,
                            y_lo..y_hi,
                            inner_iv,
                            (x_lo - h_x_i)..x_lo,
                            data_dim_iv_map,
                        );
                        let tile = IterTile::new(bounds);
                        emit_pair(neighbour, this_w, data, tile, state);
                    }
                }
            }

            // ---- E neighbour: row, col+1. ----
            if col + 1 < grid_cols_usize {
                let neighbour_idx = row * grid_cols_usize + (col + 1);
                let neighbour = body_workers[neighbour_idx];
                for &data in &all_halo_data {
                    if let Some(&h_x) = data_with_halo_x.get(&data) {
                        let h_x_i: i64 = h_x.try_into().unwrap_or(i64::MAX);
                        let bounds = order_halo_strip_bounds_by_data_dim(
                            data,
                            outer_iv,
                            y_lo..y_hi,
                            inner_iv,
                            x_hi..(x_hi + h_x_i),
                            data_dim_iv_map,
                        );
                        let tile = IterTile::new(bounds);
                        emit_pair(neighbour, this_w, data, tile, state);
                    }
                }
            }
        }
    }

    if to_insert.is_empty() {
        return node;
    }

    // Walk the ACFG once and place each group's pairs in the parent
    // Sequence containing the outer Repeat (TASK-0290 cycle 114b:
    // placement is AFTER the LAST producing Operation, not at the
    // front of the Sequence; see `prepend_strip_pairs` for the
    // rationale and the fallback rule). `to_insert` is threaded
    // through the walk as a `&mut` arg shared with the recursive
    // helper; entries are drained (removed) by `prepend_strip_pairs`
    // as it places them. After the walk, any outer_iv still in the
    // map is a sidecar/tree mismatch we tolerate silently (a
    // synthetic test or a future composition where the
    // partition_pairs entry survives a tree restructure that loses
    // the matching Repeat). No panic — the synthesis is simply a
    // no-op for that pair.
    let mut to_insert_mut = to_insert;
    prepend_strip_pairs(node, &mut to_insert_mut)
}

/// Walk `node` and insert each group's `Vec<XferPlaceholder>` into the
/// Sequence that contains the Repeat for the matching `outer_iv`.
///
/// **Placement rule** (TASK-0290 cycle 114b — architect P1 from cycle 114a):
/// for each group keyed by `outer_iv`, the synthesised Xfers are
/// inserted into the parent Sequence AFTER the LAST direct-child
/// `Operation` whose `dataflow.edges[*].data_out` is one of the halo
/// data symbols that group carries (the producer Operations on the
/// host side that load the data the halo strips will exchange).
///
/// **Why**: a worker's emitted EventList orders Push/Wait pairs by
/// source-tree order. If the synthesised `Wait` lands BEFORE the
/// host's `load_image` Operation in the root Sequence, the receiving
/// worker schedules its halo-strip Wait against a producer Push that
/// has not yet fired on the host — a real ordering defect, not just
/// aesthetic. Inserting AFTER the producer Operation orders the new
/// Push/Wait pair behind the producer's host-broadcast pairs (which
/// `splice_pushes_for_waits` placed immediately after the producer
/// via the dedupe-window path), which is the data-flow-correct order.
///
/// **Fallback**: if NO direct-child Operation in the parent Sequence
/// produces a halo data symbol for this group, the group is prepended
/// to the front of the Sequence (the pre-cycle-114b behaviour). This
/// keeps synthetic test fixtures without a top-level producer
/// (`tests/halo_strip_synth.rs` — 4 of 5 cases) green by construction:
/// their parent Sequence contains only the outer Repeat, so the
/// "insert at index 0" position is unchanged.
///
/// `to_insert` is consumed: when an outer_iv's group is placed, its
/// entry is removed. After the walk, any entries left in `to_insert`
/// were unmatched — the synthesis is silently dropped for those (a
/// malformed tree-vs-sidecar disagreement we don't loudly panic on,
/// matching the defensive skip in the synthesis loop above).
pub(super) fn prepend_strip_pairs(
    node: ACFGNode,
    to_insert: &mut BTreeMap<IterVar, Vec<XferPlaceholder>>,
) -> ACFGNode {
    match node {
        ACFGNode::Sequence(children) => {
            // Phase 1: bind each matching outer_iv to its placement
            // position in the children vector. Position = (last
            // producer index for the group's halo data symbols) + 1,
            // or 0 if no producer is found (fallback for synthetic
            // fixtures + structurally-redundant single-pass stencils).
            //
            // We use BTreeMap iteration order on `to_insert` to keep
            // the per-outer_iv visit order deterministic (numeric
            // IterVar order). Multiple groups landing at the SAME
            // position concatenate in BTreeMap order.
            let mut groups_at: BTreeMap<usize, Vec<XferPlaceholder>> = BTreeMap::new();
            let matched_ivs: Vec<IterVar> = children
                .iter()
                .filter_map(|c| {
                    if let ACFGNode::Repeat { iter_var, .. } = c {
                        if to_insert.contains_key(iter_var) {
                            return Some(*iter_var);
                        }
                    }
                    None
                })
                .collect();
            for iv in matched_ivs {
                // Pull the group out (we know it exists).
                let group = to_insert.remove(&iv).expect("checked above");
                // Halo data symbols this group carries — used to
                // identify producing Operations in the parent
                // Sequence.
                let halo_data: BTreeSet<DataId> = group.iter().map(|p| p.data).collect();
                // Find the LAST direct-child Operation that writes any
                // symbol in `halo_data`. We walk all edges (not just
                // edges[0]) — a future multi-edge DAG may carry the
                // halo data on a non-first edge, and we want the
                // placement to follow the data, not the convention of
                // edge[0].
                let mut last_producer_idx: Option<usize> = None;
                for (idx, child) in children.iter().enumerate() {
                    if let ACFGNode::Operation(op) = child {
                        let writes_halo =
                            op.dataflow.edges.iter().any(|e| {
                                e.data_out.map(|d| halo_data.contains(&d)).unwrap_or(false)
                            });
                        if writes_halo {
                            last_producer_idx = Some(idx);
                        }
                    }
                }
                let insert_pos = match last_producer_idx {
                    Some(idx) => idx + 1,
                    None => 0,
                };
                groups_at.entry(insert_pos).or_default().extend(group);
            }
            // Phase 2: recurse into each child so any nested
            // partitioned Repeat can drain its own group from
            // `to_insert`. We DO NOT recurse into the children before
            // computing `last_producer_idx` above, because the
            // recursion might restructure them — but the matched_ivs
            // and producer-index computation reads only the
            // top-level shape (Operation/Repeat at this Sequence
            // level), so the pre-recursion scan is safe.
            //
            // Recursion MUST preserve per-child arity (every
            // ACFGNode::{Sequence,Repeat,Operation,Sync,Xfer} arm in
            // `prepend_strip_pairs` returns exactly one node), so the
            // post-recursion `rewritten_children.len()` equals the
            // pre-recursion `children.len()` — keeping the
            // `last_producer_idx` indices stable across phases. If a
            // future variant of this walker ever inserted at a child
            // level the indices would silently shift; the
            // `debug_assert!` below pins the invariant
            // (TASK-0290 cycle 114b architect P1.2 hardening).
            let n_existing = children.len();
            let rewritten_children: Vec<ACFGNode> = children
                .into_iter()
                .map(|c| prepend_strip_pairs(c, to_insert))
                .collect();
            debug_assert_eq!(
                rewritten_children.len(),
                n_existing,
                "prepend_strip_pairs phase-2 recursion must preserve per-child arity \
                 (every ACFGNode arm returns exactly one node); if this fires, a future \
                 variant has begun inserting at a child level and last_producer_idx is \
                 now stale"
            );
            // Phase 3: assemble the output Sequence, splicing each
            // group's Xfers in at its computed `insert_pos`. We walk
            // children in order; before emitting the child at index
            // `k`, we drain any groups whose insert_pos == k. After
            // the last child, drain any groups whose insert_pos ==
            // n_existing (appended at the tail).
            let n_inserted: usize = groups_at.values().map(|v| v.len()).sum();
            let mut out: Vec<ACFGNode> = Vec::with_capacity(n_inserted + n_existing);
            for (k, child) in rewritten_children.into_iter().enumerate() {
                if let Some(group) = groups_at.remove(&k) {
                    for p in group {
                        out.push(ACFGNode::Xfer(p));
                    }
                }
                out.push(child);
            }
            // Tail: groups whose insert_pos == n_existing (i.e. after
            // every child). In practice the only positions that ever
            // get computed are 0..=n_existing-1 (a producer must
            // exist as a child to anchor the placement), but the
            // tail-drain keeps the code total and easy to reason
            // about. Drained in BTreeMap key order.
            for (_, group) in groups_at {
                for p in group {
                    out.push(ACFGNode::Xfer(p));
                }
            }
            ACFGNode::Sequence(out)
        }
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            block_tag,
        } => {
            // A bare Repeat (not wrapped in a Sequence) whose iter_var
            // matches a group: wrap it. In practice `build_acfg` always
            // wraps top-level statements in a Sequence, so this branch
            // is defensive; recursing into the body lets deeper
            // matches still drain.
            let new_body = Box::new(prepend_strip_pairs(*body, to_insert));
            ACFGNode::Repeat {
                iter_var,
                range,
                body: new_body,
                block_tag,
            }
        }
        leaf @ (ACFGNode::Operation(_) | ACFGNode::Sync(_) | ACFGNode::Xfer(_)) => leaf,
    }
}
