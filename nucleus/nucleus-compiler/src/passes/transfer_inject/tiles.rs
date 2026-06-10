use super::*;

// --------------------------------------------------------------------
// TASK-0341.02.02.01.{02,03} cycle 213 — cumulative-array
// partition-band exchange (16-jacobi/distributed)
// --------------------------------------------------------------------

/// Build the SENDER write-band tile for a cumulative data symbol's
/// transfer from compute worker `src`. (TASK-0341.02.02.01.03 cycle 213)
///
/// The tile has one bound per data dim (positional `bounds[i] <->
/// dims[i]`, the `wait_slice` convention):
/// - A dim covered by a PARTITIONED iv (per `data_dim_iv_map`) gets the
///   SRC worker's WRITE BAND `partition_ranges[iv][src]` — NOT the
///   halo-expanded read range (the architect's write-band-not-halo
///   point: halo-expanded bands OVERLAP across workers and a slice-paste
///   COPY over them would double-write the boundary rows).
/// - Every other dim gets the FULL range `0..dims[i]` with the dim's
///   observed iv (or a fallback iv if none observed — the iv is
///   decorative for `wait_slice`, only the range is load-bearing).
///
/// For 16-jacobi `field[5][8][8]` × `partition=rows(y)` from w1 (band
/// 1..3): `[(t, 0..5), (y, 1..3), (x, 0..8)]`. Returns `None` if the
/// data has no partitioned dim on this worker (then the caller leaves
/// the tile unchanged — should not happen for a partitioned cumulative
/// array, but stays total).
pub(super) fn cumulative_band_bounds(
    data: DataId,
    src: WorkerId,
    dims: &[i64],
    data_dim_iv_map: &BTreeMap<DataId, Vec<BTreeSet<IterVar>>>,
    partition_ranges: &BTreeMap<IterVar, BTreeMap<WorkerId, std::ops::Range<i64>>>,
) -> Option<Vec<(IterVar, std::ops::Range<i64>)>> {
    let per_dim = data_dim_iv_map.get(&data)?;
    // The data must have a per-dim iv map covering all its dims for the
    // positional construction to be sound.
    if per_dim.len() != dims.len() {
        return None;
    }
    let mut bounds: Vec<(IterVar, std::ops::Range<i64>)> = Vec::with_capacity(dims.len());
    let mut saw_band = false;
    for (d, iv_set) in per_dim.iter().enumerate() {
        // Find the (unique) partitioned iv covering this dim, if any.
        let partitioned: Vec<IterVar> = iv_set
            .iter()
            .copied()
            .filter(|iv| partition_ranges.contains_key(iv))
            .collect();
        if partitioned.len() == 1 {
            let iv = partitioned[0];
            if let Some(band) = partition_ranges.get(&iv).and_then(|m| m.get(&src)).cloned() {
                bounds.push((iv, band));
                saw_band = true;
                continue;
            }
        }
        // Full range for this dim. Use the first observed iv as the
        // decorative carrier; fall back to the partitioned iv (just to
        // have *some* iv) if the dim observed none. The iv is never
        // consulted by `wait_slice` (only the range is), so any iv is
        // sound for a FULL axis.
        let carrier = iv_set
            .iter()
            .copied()
            .next()
            .or_else(|| partition_ranges.keys().copied().next())
            .unwrap_or(IterVar(0));
        bounds.push((carrier, 0..dims[d]));
    }
    if saw_band {
        Some(bounds)
    } else {
        None
    }
}

/// Rewrite every cumulative-array Xfer's tile to the SENDER write band
/// (TASK-0341.02.02.01.03 cycle 213). Applies to BOTH the w2w exchange
/// AND the worker->host gather (architect P1-2: the host gather of
/// identical full slices is itself an xN source; copying disjoint write
/// bands instead reconstructs the full array on the host). Runs AFTER
/// the partition / halo / strip passes so it OVERRIDES any halo-expanded
/// or whole-array tile those passes set on a cumulative symbol.
///
/// The compute (band-owning) worker is the Xfer's `src` for a gather/
/// w2w-send. For a cumulative symbol every transfer is a band-send from
/// its producing compute worker, so `src` is always the band owner.
pub(super) fn rewrite_cumulative_band_tiles(
    node: ACFGNode,
    cumulative_data: &BTreeSet<DataId>,
    partition_ranges: &BTreeMap<IterVar, BTreeMap<WorkerId, std::ops::Range<i64>>>,
    data_dim_iv_map: &BTreeMap<DataId, Vec<BTreeSet<IterVar>>>,
    data_dims: &BTreeMap<DataId, Vec<i64>>,
) -> Result<ACFGNode, TransferInjectError> {
    match node {
        ACFGNode::Xfer(mut x) => {
            if cumulative_data.contains(&x.data) {
                if let Some(dims) = data_dims.get(&x.data) {
                    if let Some(bounds) = cumulative_band_bounds(
                        x.data,
                        x.src,
                        dims,
                        data_dim_iv_map,
                        partition_ranges,
                    ) {
                        x.tile = IterTile::new(bounds);
                    } else if !partition_ranges.is_empty() {
                        // FAIL-LOUD on the genuine xN-risk shape only
                        // (TASK-0366 cycle-214; cycle-213 architect P3).
                        //
                        // A `None` from `cumulative_band_bounds` means we
                        // could NOT derive a per-src write band for this
                        // cumulative symbol. There are TWO distinct ways to
                        // get here, and only ONE is a defect:
                        //
                        //   (A) `partition_ranges` is NON-EMPTY (some loop
                        //       carries `partition=`) yet no partitioned iv
                        //       covers any dim of THIS cumulative array — so
                        //       the array is REPLICATED whole across the
                        //       partition workers. Keeping the whole-array
                        //       tile would make the host gather (and any w2w
                        //       exchange) copy N identical full replicas,
                        //       silently re-introducing the xN double-count
                        //       this pass exists to remove. That is a
                        //       miscompile — fail at COMPILE time (recurring
                        //       defect pattern #3: silent fallback -> typed
                        //       error) so the write-band derivation gets
                        //       extended rather than shipping xN-wrong output.
                        //
                        //   (B) `partition_ranges` is EMPTY (no `partition=`
                        //       anywhere, e.g. 11-game-of-life/pipelined:
                        //       `grid` is cumulative and cross-worker via the
                        //       async double-buffer channel, but there is a
                        //       SINGLE compute worker owning all of `grid`).
                        //       There is no partition to double-count against,
                        //       so the WHOLE-ARRAY tile is the CORRECT
                        //       transfer shape. This is the `else` fall-
                        //       through below — keep the tile unchanged,
                        //       silently. (The cycle-213 comment claimed this
                        //       case "short-circuits to a no-op"; it does not
                        //       short-circuit — it reaches here and the
                        //       whole-array tile is simply correct.)
                        // The user-facing `message` is tracker-ID-free
                        // (TASK-0455.06: tracker IDs stay in code comments
                        // and the variant docstring, not in surfaced
                        // diagnostics). The internal forward-link is
                        // TASK-0366 — see the `CumulativeWholeArrayFallback`
                        // variant doc on `TransferInjectError`.
                        return Err(TransferInjectError::CumulativeWholeArrayFallback {
                            data: x.data,
                            src: x.src,
                            message: format!(
                                "cumulative data {:?} (src {:?}) is accumulated across iterations \
                                 and a partition is active, but no partitioned loop variable \
                                 covers any of its dimensions on this worker. Keeping the \
                                 whole-array transfer here would silently re-introduce an xN \
                                 double-count (host gather / worker-to-worker exchange of N \
                                 identical full slices). This is a conservative guard: a \
                                 single-worker cumulative array sitting alongside an unrelated \
                                 partitioned array is over-rejected on purpose. Fix: give this \
                                 array a partitioned loop variable on one of its dimensions, or \
                                 (if it is genuinely replicated) extend the write-band derivation \
                                 so the per-worker band is recoverable.",
                                x.data, x.src
                            ),
                        });
                    }
                    // else: partition_ranges empty (case B) — unpartitioned
                    // cumulative symbol; the whole-array tile already on `x`
                    // is correct. Leave it unchanged.
                }
            }
            Ok(ACFGNode::Xfer(x))
        }
        ACFGNode::Sequence(children) => Ok(ACFGNode::Sequence(
            children
                .into_iter()
                .map(|c| {
                    rewrite_cumulative_band_tiles(
                        c,
                        cumulative_data,
                        partition_ranges,
                        data_dim_iv_map,
                        data_dims,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?,
        )),
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            block_tag,
            break_cond,
        } => Ok(ACFGNode::Repeat {
            iter_var,
            range,
            body: Box::new(rewrite_cumulative_band_tiles(
                *body,
                cumulative_data,
                partition_ranges,
                data_dim_iv_map,
                data_dims,
            )?),
            block_tag,
            // Structure-preserving rewrite (only `body` changes); carry
            // the `for..until` halt predicate through unchanged.
            break_cond,
        }),
        leaf @ (ACFGNode::Operation(_) | ACFGNode::Sync(_)) => Ok(leaf),
    }
}

/// Hoist cumulative-array worker-to-worker `Xfer`s out of the partition
/// spatial loops to the enclosing `Repeat` body (TASK-0341.02.02.01.02
/// cycle 213).
///
/// For 16-jacobi/distributed the w2w `field` Push/Wait are emitted
/// INSIDE `for x` inside `for y` (the partition iv), giving each worker
/// `band_rows * x_span` exchanges — unequal across workers (bands differ
/// in size) — which deadlocks on the single-element rendezvous slot
/// (cycle 211b/212 empirical). Hoisting them to the `for t` body (once
/// per t, equal counts on every worker) with SEND-then-RECV ordering
/// (Push then Wait, both after the band compute) fixes the deadlock: the
/// non-blocking Slot push lets every worker fill its outgoing slots
/// before any worker waits on an incoming one.
///
/// # The transform
///
/// In every `Sequence`, when a child is a `Repeat` whose `iter_var` is a
/// PARTITION iv (in `partition_ranges`), STRIP every cumulative-data
/// `Xfer` from that Repeat's subtree (recursively) and re-insert them in
/// the enclosing Sequence IMMEDIATELY AFTER the Repeat: all Pushes
/// first, then all Waits (send-then-recv). The Operation (band compute)
/// and the loop structure stay; only the cumulative w2w transfers move.
///
/// Worker->host gather Xfers of the cumulative symbol live OUTSIDE any
/// partition-iv Repeat (at the worker's top-level sequence, after the
/// outer Repeat), so they are NOT inside a partition-iv loop and are
/// left in place (their tile is already the write band from
/// `rewrite_cumulative_band_tiles`).
pub(super) fn hoist_cumulative_w2w_to_repeat_body(
    node: ACFGNode,
    cumulative_data: &BTreeSet<DataId>,
    partition_ranges: &BTreeMap<IterVar, BTreeMap<WorkerId, std::ops::Range<i64>>>,
) -> ACFGNode {
    match node {
        ACFGNode::Sequence(children) => {
            let mut out: Vec<ACFGNode> = Vec::with_capacity(children.len());
            for child in children {
                // Recurse first so nested Sequences (e.g. the for-t body)
                // are processed.
                let child =
                    hoist_cumulative_w2w_to_repeat_body(child, cumulative_data, partition_ranges);
                match child {
                    ACFGNode::Repeat {
                        iter_var,
                        range,
                        body,
                        block_tag,
                        break_cond,
                    } if partition_ranges.contains_key(&iter_var) => {
                        // Strip cumulative w2w Xfers from the partition
                        // Repeat's subtree; collect them.
                        let mut stripped: Vec<XferPlaceholder> = Vec::new();
                        let body = strip_cumulative_xfers(*body, cumulative_data, &mut stripped);
                        out.push(ACFGNode::Repeat {
                            iter_var,
                            range,
                            body: Box::new(body),
                            block_tag,
                            // Structure-preserving (only `body` changes);
                            // carry the halt predicate through unchanged.
                            break_cond,
                        });
                        // SEND-then-RECV: all Pushes, then all Waits.
                        // Within each role preserve discovery order
                        // (deterministic — pre-order strip).
                        for x in stripped.iter().filter(|x| x.role == XferRole::Push) {
                            out.push(ACFGNode::Xfer(x.clone()));
                        }
                        for x in stripped.iter().filter(|x| x.role == XferRole::Wait) {
                            out.push(ACFGNode::Xfer(x.clone()));
                        }
                    }
                    other => out.push(other),
                }
            }
            ACFGNode::Sequence(out)
        }
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            block_tag,
            break_cond,
        } => ACFGNode::Repeat {
            iter_var,
            range,
            body: Box::new(hoist_cumulative_w2w_to_repeat_body(
                *body,
                cumulative_data,
                partition_ranges,
            )),
            block_tag,
            // Structure-preserving rewrite (only `body` changes); carry
            // the `for..until` halt predicate through unchanged.
            break_cond,
        },
        leaf => leaf,
    }
}

/// Remove every cumulative-data `Xfer` from `node`'s subtree, appending
/// each removed placeholder (in pre-order) to `out`. Returns the subtree
/// with those Xfers excised. Used by
/// [`hoist_cumulative_w2w_to_repeat_body`] to lift the in-`for x` w2w
/// exchange up to the `for t` body. Sync / Operation nodes are kept;
/// only `Xfer` nodes whose `data` is cumulative are pulled out.
pub(super) fn strip_cumulative_xfers(
    node: ACFGNode,
    cumulative_data: &BTreeSet<DataId>,
    out: &mut Vec<XferPlaceholder>,
) -> ACFGNode {
    match node {
        ACFGNode::Xfer(x) if cumulative_data.contains(&x.data) => {
            out.push(x);
            // Replace with an empty Sequence (flattened away by the
            // projection / downstream; an empty Sequence is inert).
            ACFGNode::Sequence(Vec::new())
        }
        ACFGNode::Sequence(children) => {
            let mut kept: Vec<ACFGNode> = Vec::with_capacity(children.len());
            for c in children {
                let c = strip_cumulative_xfers(c, cumulative_data, out);
                // Drop the inert empty-Sequence placeholders left by a
                // stripped Xfer so the body stays clean.
                if matches!(&c, ACFGNode::Sequence(v) if v.is_empty()) {
                    continue;
                }
                kept.push(c);
            }
            ACFGNode::Sequence(kept)
        }
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            block_tag,
            break_cond,
        } => ACFGNode::Repeat {
            iter_var,
            range,
            body: Box::new(strip_cumulative_xfers(*body, cumulative_data, out)),
            block_tag,
            // Structure-preserving rewrite (only `body` changes); carry
            // the `for..until` halt predicate through unchanged.
            break_cond,
        },
        leaf => leaf,
    }
}

// --------------------------------------------------------------------
// TASK-0263 Stage 2 halo extension
// --------------------------------------------------------------------

/// Extend each [`XferPlaceholder`]'s `tile` axis range by the halo
/// width inferred for `(consumer_kernel, iter_var)` along that axis
/// (TASK-0263 Stage 2).
///
/// ### Why this is the right sink for halo widths
///
/// By the time `rewrite_partition_tiles` has run, every Xfer's tile
/// reflects the COMPUTE worker's partition slice (or the source range
/// when no partition applies). The halo is an additive correction:
/// "the kernel reading the slice ALSO reads `halo` rows of
/// neighbouring data". Applied here, the halo extension composes with
/// any partition policy (rows/blocks2d/workers) without each policy
/// having to know about halo.
///
/// ### Sidecar shape and the (DataId -> consumer KernelIds) join
///
/// `halo_widths` is keyed by `(KernelId, IterVar) -> u64`. An
/// XferPlaceholder carries `(DataId, IterVar)` in its tile. We bridge
/// the two by walking the ACFG once to collect, for each `DataId`, the
/// set of kernel ids that READ it (the `data_in` of every
/// `DataflowEdge`). The halo on a `(DataId, IterVar)` is then the
/// MAX of `halo_widths[kid][iv]` across all consumer kids — the union
/// of every reader's halo requirement. This is the only sound merge:
/// each transfer is one slice, used by every consumer at the
/// destination; the slice must cover the union of reads.
///
/// In practice every in-tree example has at most one consumer kernel
/// per data symbol; the max-across-consumers reduces to the single
/// entry. The variant is documented for correctness when a future
/// algorithm reads the same symbol from two different kernels.
///
/// ### Clamp policy
///
/// Extension is `lo' = lo - halo, hi' = hi + halo`. We clamp against
/// the iter-var's algorithm-source loop range expanded by halo —
/// `[source_lo - halo, source_hi + halo)`. That bounds the legitimate
/// data extent the loop would read in absence of partitioning. For
/// schedules where the per-worker band already abuts the source range
/// (the corner workers), the clamp is a no-op; for non-corner workers
/// it is always a no-op (the extension stays strictly inside the source
/// range expanded by halo).
///
/// Why not clamp against the data symbol's `ResolvedType::dims`: that
/// is the right semantic for a STRICTER clamp (don't read beyond data
/// extent), and is a natural follow-up. For every in-tree example, the
/// algorithm declares data wide enough that the (loop range ± halo)
/// stays within the data extent, so the looser clamp here is correct.
/// Filed as a future tightening on TASK-0263.
///
/// ### Bare-iv halo = 0 contract (forward-carry from TASK-0260)
///
/// `halo_widths` records an explicit `0` entry for every
/// `(kernel, iv)` the detector inspected with a bare-iv index. We
/// treat `0` as "no extension needed" (the natural arithmetic
/// identity). This honours the TASK-0260 Stage 1 lenient contract.
///
/// ### No-op when `halo_widths` is empty
///
/// An empty sidecar — every pre-Stage-2 example — short-circuits at
/// the top: no walk, no allocation, the input is returned unchanged.
pub(super) fn extend_xfer_tiles_for_halo(
    node: ACFGNode,
    halo_widths: &BTreeMap<KernelId, BTreeMap<IterVar, u64>>,
) -> ACFGNode {
    if halo_widths.is_empty() {
        return node;
    }
    // Build the (DataId -> Set<KernelId>) consumer index by walking
    // the ACFG once. Operations' DataflowEdge::data_in lists every
    // DataId the edge.kernel reads.
    let mut consumers: BTreeMap<DataId, BTreeSet<KernelId>> = BTreeMap::new();
    collect_data_consumers(&node, &mut consumers);

    // Build the (IterVar -> source-loop-range) index by walking the
    // ACFG's Repeat nodes. The first occurrence wins (matches
    // `rewrite_partition_tiles`'s nest-order convention); a future
    // strip-mined inner Repeat that reuses the same IterVar would not
    // overwrite the outer source range.
    let mut source_ranges: BTreeMap<IterVar, std::ops::Range<i64>> = BTreeMap::new();
    collect_iter_var_source_ranges(&node, &mut source_ranges);

    extend_xfer_tiles_inner(node, halo_widths, &consumers, &source_ranges)
}

/// Walk `node` and union every `Operation`'s `data_in` into the
/// `(DataId -> KernelId)` consumer index. Read-only walk.
///
/// The kernel id we attribute to a read is the EDGE's kernel
/// (`edge.kernel`), not the enclosing `Operation.kernel` — they are
/// the same in the M1 single-edge-per-Operation shape, but explicit
/// is safer if a future multi-edge DAG lands.
pub(super) fn collect_data_consumers(
    node: &ACFGNode,
    out: &mut BTreeMap<DataId, BTreeSet<KernelId>>,
) {
    match node {
        ACFGNode::Operation(op) => {
            for edge in &op.dataflow.edges {
                for d in &edge.data_in {
                    out.entry(*d).or_default().insert(edge.kernel);
                }
            }
        }
        ACFGNode::Repeat { body, .. } => collect_data_consumers(body, out),
        ACFGNode::Sequence(children) => {
            for c in children {
                collect_data_consumers(c, out);
            }
        }
        ACFGNode::Sync(_) | ACFGNode::Xfer(_) => {}
    }
}

/// Walk `node` and record, for each `Repeat::iter_var`, its source
/// `range`. First occurrence wins. Read-only walk.
///
/// "First" is the outermost occurrence under DFS pre-order. A future
/// `block_transform` strip-mine that reuses an IterVar across full +
/// partial split would land its inner-tile Repeats deeper in the tree;
/// taking the outer occurrence keeps the halo clamp anchored on the
/// SOURCE loop range (the algorithm's `for y : 1..H-1`), not on a
/// downstream-synthesised inner-tile range.
pub(super) fn collect_iter_var_source_ranges(
    node: &ACFGNode,
    out: &mut BTreeMap<IterVar, std::ops::Range<i64>>,
) {
    match node {
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            ..
        } => {
            out.entry(*iter_var).or_insert_with(|| range.clone());
            collect_iter_var_source_ranges(body, out);
        }
        ACFGNode::Sequence(children) => {
            for c in children {
                collect_iter_var_source_ranges(c, out);
            }
        }
        ACFGNode::Operation(_) | ACFGNode::Sync(_) | ACFGNode::Xfer(_) => {}
    }
}

/// Recursive worker for [`extend_xfer_tiles_for_halo`].
pub(super) fn extend_xfer_tiles_inner(
    node: ACFGNode,
    halo_widths: &BTreeMap<KernelId, BTreeMap<IterVar, u64>>,
    consumers: &BTreeMap<DataId, BTreeSet<KernelId>>,
    source_ranges: &BTreeMap<IterVar, std::ops::Range<i64>>,
) -> ACFGNode {
    match node {
        ACFGNode::Xfer(mut x) => {
            // Look up the consumer kernels for this data symbol. If
            // none recorded (the data is never read in the ACFG —
            // e.g. an output written but not re-consumed), no halo
            // applies. Skip.
            let Some(kernel_ids) = consumers.get(&x.data) else {
                return ACFGNode::Xfer(x);
            };
            let mut new_bounds: Vec<(IterVar, std::ops::Range<i64>)> =
                Vec::with_capacity(x.tile.bounds.len());
            for (iv, range) in &x.tile.bounds {
                // Max halo across consumer kernels for this iv.
                let halo: u64 = kernel_ids
                    .iter()
                    .filter_map(|kid| halo_widths.get(kid))
                    .filter_map(|per_iv| per_iv.get(iv))
                    .copied()
                    .max()
                    .unwrap_or(0);
                if halo == 0 {
                    new_bounds.push((*iv, range.clone()));
                    continue;
                }
                // halo is u64; convert to i64. For all in-tree
                // examples halo widths are small (1 to a few), so the
                // overflow path is academic — we still guard with
                // try_into + saturating fallback to keep the function
                // total.
                let halo_i: i64 = halo.try_into().unwrap_or(i64::MAX);
                // Extend.
                let ext_lo = range.start.saturating_sub(halo_i);
                let ext_hi = range.end.saturating_add(halo_i);
                // Clamp to the iter-var's source loop range expanded by
                // halo. (See module docs for the rationale on this
                // looser clamp vs the data-extent clamp.)
                let (clamp_lo, clamp_hi) = match source_ranges.get(iv) {
                    Some(src) => (
                        src.start.saturating_sub(halo_i),
                        src.end.saturating_add(halo_i),
                    ),
                    // No source range recorded for this iv — happens
                    // if an Xfer carries an iv that has no Repeat in
                    // the ACFG (synthetic test fixtures). Skip the
                    // clamp; the extension is still applied.
                    None => (i64::MIN, i64::MAX),
                };
                let new_lo = ext_lo.max(clamp_lo);
                let new_hi = ext_hi.min(clamp_hi);
                new_bounds.push((*iv, new_lo..new_hi));
            }
            x.tile = IterTile::new(new_bounds);
            ACFGNode::Xfer(x)
        }
        ACFGNode::Sequence(children) => ACFGNode::Sequence(
            children
                .into_iter()
                .map(|c| extend_xfer_tiles_inner(c, halo_widths, consumers, source_ranges))
                .collect(),
        ),
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            block_tag,
            break_cond,
        } => ACFGNode::Repeat {
            iter_var,
            range,
            body: Box::new(extend_xfer_tiles_inner(
                *body,
                halo_widths,
                consumers,
                source_ranges,
            )),
            block_tag,
            // Structure-preserving rewrite (only `body` changes); carry
            // the `for..until` halt predicate through unchanged.
            break_cond,
        },
        leaf @ (ACFGNode::Operation(_) | ACFGNode::Sync(_)) => leaf,
    }
}
