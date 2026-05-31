//! Inline unit tests for `order_halo_strip_bounds_by_data_dim`.
//!
//! TASK-0315: pin the helper's branch selection directly. The
//! integration tests in `tests/halo_strip_synth.rs` (task0306_ac3 /
//! ac4) exercise the helper via `inject_transfers` end-to-end and
//! observe the EMIT shape; these direct-call tests prove the
//! helper's branch ordering as a unit, so a future refactor of
//! `inject_halo_strip_xfers` that bypasses or short-circuits the
//! helper would still be caught by these unit pins.

use super::*;
use std::collections::{BTreeMap, BTreeSet};

fn iv_set(ivs: &[IterVar]) -> BTreeSet<IterVar> {
    ivs.iter().copied().collect()
}

/// AC#2 (positive arm): with a populated `data_dim_iv_map` entry,
/// the helper returns dim-ordered bounds — proving the fall-back
/// branch was NOT taken. Indices `[outer_iv][inner_iv]` → canonical
/// outer-leading emit.
#[test]
fn task0315_outer_leading_takes_canonical_path_not_fallback() {
    let outer_iv = IterVar(7);
    let inner_iv = IterVar(8);
    let data = DataId(99);
    let mut map: BTreeMap<DataId, Vec<BTreeSet<IterVar>>> = BTreeMap::new();
    map.insert(data, vec![iv_set(&[outer_iv]), iv_set(&[inner_iv])]);

    let got = order_halo_strip_bounds_by_data_dim(data, outer_iv, 8..9, inner_iv, 0..8, &map);

    assert_eq!(
        got,
        vec![(outer_iv, 8..9), (inner_iv, 0..8)],
        "TASK-0315 AC#2: outer-leading data layout MUST emit canonical \
         outer-first order via the data-dim consultation branch, not the \
         default-order fall-back (which would happen to coincide here, \
         but the swapped-layout test below proves the canonical branch \
         actually fires).",
    );
}

/// AC#2 (cross-check): with a SWAPPED dim layout (inner_iv at dim 0,
/// outer_iv at dim 1) the helper MUST flip emit order. Default-order
/// fall-back would return `[(outer_iv, ...), (inner_iv, ...)]` — a
/// different vector — so this directly distinguishes the canonical
/// path from the fall-back.
#[test]
fn task0315_inner_leading_flips_order_proving_non_fallback() {
    let outer_iv = IterVar(7);
    let inner_iv = IterVar(8);
    let data = DataId(99);
    let mut map: BTreeMap<DataId, Vec<BTreeSet<IterVar>>> = BTreeMap::new();
    // dim 0 = inner_iv, dim 1 = outer_iv (inner-axis-leading layout).
    map.insert(data, vec![iv_set(&[inner_iv]), iv_set(&[outer_iv])]);

    let got = order_halo_strip_bounds_by_data_dim(data, outer_iv, 8..9, inner_iv, 0..8, &map);

    assert_eq!(
        got,
        vec![(inner_iv, 0..8), (outer_iv, 8..9)],
        "TASK-0315 AC#2: inner-axis-leading layout MUST flip to \
         inner-first emit. The fall-back branch would have returned \
         [(outer_iv, ...), (inner_iv, ...)] — its return value \
         differs from this expected output, so a passing assertion \
         here is direct evidence the canonical (non-fall-back) \
         branch fired.",
    );
}

/// Fall-back branch: `Some(empty)` per-dim Vec ⇒ default order. This
/// is the path the cycle-133 NUC_TRACE diagnostic now reports. The
/// behaviour is unchanged from cycle 133 — pinned here so a future
/// edit to the fall-back cannot silently change its return shape.
#[test]
fn task0315_some_empty_falls_back_to_default_order() {
    let outer_iv = IterVar(7);
    let inner_iv = IterVar(8);
    let data = DataId(99);
    let mut map: BTreeMap<DataId, Vec<BTreeSet<IterVar>>> = BTreeMap::new();
    map.insert(data, Vec::new());

    let got = order_halo_strip_bounds_by_data_dim(data, outer_iv, 8..9, inner_iv, 0..8, &map);

    assert_eq!(
        got,
        vec![(outer_iv, 8..9), (inner_iv, 0..8)],
        "TASK-0315: Some(empty) takes fall-back; default order is \
         outer-leading.",
    );
}

/// Fall-back branch: data missing from the map ⇒ default order (the
/// `None` arm). Production callers never reach this path; pinned for
/// the synthetic-fixture safety net.
#[test]
fn task0315_missing_data_falls_back_to_default_order() {
    let outer_iv = IterVar(7);
    let inner_iv = IterVar(8);
    let data = DataId(99);
    let map: BTreeMap<DataId, Vec<BTreeSet<IterVar>>> = BTreeMap::new();

    let got = order_halo_strip_bounds_by_data_dim(data, outer_iv, 8..9, inner_iv, 0..8, &map);

    assert_eq!(
        got,
        vec![(outer_iv, 8..9), (inner_iv, 0..8)],
        "TASK-0315: missing entry takes fall-back; default order is \
         outer-leading.",
    );
}

// ----------------------------------------------------------------
// TASK-0317: silent-sibling pins for
// `compute_partition_bounds_with_dim_prefix`.
//
// Same shape as the TASK-0315 inline tests above. The helper is
// the canonical-path arm of `rewrite_partition_tiles_inner`; the
// `None` arm now triggers a NUC_TRACE diagnostic in the caller
// (the fall-back observability addition this task lands).
//
// We pin: (a) the data-dim-aware canonical Some-arm, (b) the
// missing-entry None arm that drives the caller's nest-order
// fall-back + trace emit, (c) the empty-per-dim None arm
// (parallel to TASK-0315's Some(empty) twin), and (d) the
// sparse-coverage whole-array drop.
// ----------------------------------------------------------------

#[allow(clippy::type_complexity)]
fn make_partition_ranges(
    entries: &[(IterVar, &[(WorkerId, std::ops::Range<i64>)])],
) -> BTreeMap<IterVar, BTreeMap<WorkerId, std::ops::Range<i64>>> {
    let mut out: BTreeMap<IterVar, BTreeMap<WorkerId, std::ops::Range<i64>>> = BTreeMap::new();
    for (iv, per_worker) in entries {
        let map: BTreeMap<_, _> = per_worker.iter().map(|(w, r)| (*w, r.clone())).collect();
        out.insert(*iv, map);
    }
    out
}

/// AC: canonical Some-arm — `data_dim_iv_map` indexed [outer][inner]
/// with both partitioned ⇒ returns dim-ordered bounds. Caller does
/// NOT take the fall-back; no trace fires.
#[test]
fn task0317_canonical_path_returns_dim_ordered_bounds_no_fallback() {
    let outer_iv = IterVar(7);
    let inner_iv = IterVar(8);
    let data = DataId(99);
    let worker = WorkerId(2);
    let mut map: BTreeMap<DataId, Vec<BTreeSet<IterVar>>> = BTreeMap::new();
    map.insert(data, vec![iv_set(&[outer_iv]), iv_set(&[inner_iv])]);

    let partition_ranges = make_partition_ranges(&[
        (outer_iv, &[(worker, 8..16)]),
        (inner_iv, &[(worker, 0..8)]),
    ]);

    let got = compute_partition_bounds_with_dim_prefix(data, &map, &partition_ranges, worker);

    assert_eq!(
        got,
        Some(vec![(outer_iv, 8..16), (inner_iv, 0..8)]),
        "TASK-0317 canonical: both dims covered by partitioned ivs in \
         nest-prefix order ⇒ dim-ordered bounds, no fall-back.",
    );
}

/// AC: None arm — `data_dim_iv_map` missing entry for the data
/// symbol ⇒ caller takes nest-order fall-back + emits NUC_TRACE.
/// This is the arm the trace is observability-instrumenting.
#[test]
fn task0317_missing_entry_returns_none_drives_fallback() {
    let outer_iv = IterVar(7);
    let data = DataId(99);
    let worker = WorkerId(2);
    let map: BTreeMap<DataId, Vec<BTreeSet<IterVar>>> = BTreeMap::new();
    let partition_ranges = make_partition_ranges(&[(outer_iv, &[(worker, 8..16)])]);

    let got = compute_partition_bounds_with_dim_prefix(data, &map, &partition_ranges, worker);

    assert_eq!(
        got, None,
        "TASK-0317 None-arm: missing data_dim_iv_map entry returns \
         None; rewrite_partition_tiles_inner's fall-back arm fires \
         on this case and emits the NUC_TRACE diagnostic.",
    );
}

/// AC: None arm — `data_dim_iv_map` records data with empty per-dim
/// Vec (synthetic fixtures via DataflowEdge::new) ⇒ returns None
/// via the `per_dim.is_empty()` early-out in
/// `compute_partition_bounds_with_dim_prefix`. Caller
/// takes the same fall-back as the missing-entry case.
#[test]
fn task0317_empty_per_dim_returns_none_drives_fallback() {
    let outer_iv = IterVar(7);
    let data = DataId(99);
    let worker = WorkerId(2);
    let mut map: BTreeMap<DataId, Vec<BTreeSet<IterVar>>> = BTreeMap::new();
    map.insert(data, Vec::new());
    let partition_ranges = make_partition_ranges(&[(outer_iv, &[(worker, 8..16)])]);

    let got = compute_partition_bounds_with_dim_prefix(data, &map, &partition_ranges, worker);

    assert_eq!(
        got, None,
        "TASK-0317 None-arm (twin): empty per-dim Vec returns None \
         via the explicit is_empty() early-out. Caller fall-back \
         trace fires on this arm too.",
    );
}

/// AC: sparse-coverage whole-array drop — partitioned iv covers
/// dim 1 but not dim 0 (a hole at dim 0 followed by a covered
/// dim 1 violates the contiguous-prefix invariant). Returns
/// `Some(Vec::new())` per the safe-drop policy in
/// `compute_partition_bounds_with_dim_prefix`.
#[test]
fn task0317_sparse_coverage_drops_to_whole_array() {
    let outer_iv = IterVar(7);
    let inner_iv = IterVar(8);
    let unpart_iv = IterVar(42);
    let data = DataId(99);
    let worker = WorkerId(2);
    let mut map: BTreeMap<DataId, Vec<BTreeSet<IterVar>>> = BTreeMap::new();
    // dim 0 = unpartitioned (k), dim 1 = inner_iv (x, partitioned).
    // outer_iv is partitioned but doesn't index this data.
    map.insert(data, vec![iv_set(&[unpart_iv]), iv_set(&[inner_iv])]);
    let partition_ranges = make_partition_ranges(&[
        (outer_iv, &[(worker, 8..16)]),
        (inner_iv, &[(worker, 0..8)]),
    ]);

    let got = compute_partition_bounds_with_dim_prefix(data, &map, &partition_ranges, worker);

    assert_eq!(
        got,
        Some(Vec::new()),
        "TASK-0317 sparse: dim 0 has no partitioned iv covering it \
         (k is unpartitioned), dim 1 does. Sparse coverage triggers \
         the safe whole-array drop per compute_partition_bounds_with_\
         dim_prefix's hole-after-cover policy.",
    );
}

// ----------------------------------------------------------------
// TASK-0373: OPAQUE-dim attribution for a data-dependent (gather)
// index. These pin the mis-attribution fix at the
// `collect_data_dim_iv_map` layer — the inner ivs of a gather index
// (`i`, `k` from `col_idx[i][k]`) must NOT land on the outer
// gathered array's dim, so that array falls to whole-array
// broadcast.
// ----------------------------------------------------------------

/// Build a single-Operation ACFG node carrying the given
/// `data_in_access` accesses (helper for the TASK-0373 opaque-dim
/// tests). Mirrors the canonical `Operation { kernel, workers,
/// dataflow }` shape `build_acfg` produces, minus the bits
/// `collect_data_dim_iv_map` ignores.
fn op_node_with_accesses(accesses: Vec<DataAccess>) -> ACFGNode {
    let data_in: Vec<DataId> = accesses.iter().map(|a| a.data).collect();
    ACFGNode::Operation(Operation {
        kernel: KernelId(0),
        workers: [WorkerId(0)].into_iter().collect(),
        dataflow: crate::acfg::DataflowDag {
            edges: vec![crate::acfg::DataflowEdge {
                data_in,
                kernel: KernelId(0),
                data_out: None,
                data_in_access: accesses,
                data_out_access: None,
                args: Vec::new(),
            }],
        },
    })
}

/// AC#2: for the gather `x[col_idx[i][k]]`, the OUTER array `x`'s
/// dim 0 is recorded OPAQUE (empty iv set) — the inner ivs `{i, k}`
/// are NOT attributed to it — so `compute_partition_bounds_with_dim_
/// prefix` returns whole-array broadcast for `x` under
/// `partition=workers(i)`. The pre-TASK-0373 "defensive descent"
/// would have recorded `{i, k}` on `x` dim 0 and emitted a WRONG
/// i-band slice. The index array `col_idx[i][k]` itself stays
/// iv-affine (`{i}`, `{k}`) so it i-bands like `val`.
#[test]
fn task0373_gather_outer_array_dim_is_opaque_not_iv_attributed() {
    let i_iv = IterVar(1);
    let k_iv = IterVar(2);
    let x = DataId(10);
    let col_idx = DataId(11);
    let mut name_iter_vars: BTreeMap<String, IterVar> = BTreeMap::new();
    name_iter_vars.insert("i".to_string(), i_iv);
    name_iter_vars.insert("k".to_string(), k_iv);

    // x[col_idx[i][k]] — a single dim whose index is the gather
    // DataRef col_idx[i][k].
    let x_access = DataAccess {
        data: x,
        indices: vec![IrExpr::DataRef(crate::algo::ir::IndexedRef {
            name: "col_idx".to_string(),
            indices: vec![
                IrExpr::Ident("i".to_string()),
                IrExpr::Ident("k".to_string()),
            ],
        })],
    };
    // col_idx[i][k] — the index array, iv-affine on i,k (this is the
    // access build_acfg's TASK-0373 recursion now also records).
    let col_idx_access = DataAccess {
        data: col_idx,
        indices: vec![
            IrExpr::Ident("i".to_string()),
            IrExpr::Ident("k".to_string()),
        ],
    };
    let node = op_node_with_accesses(vec![x_access, col_idx_access]);

    let map = collect_data_dim_iv_map(&node, &name_iter_vars);

    // x dim 0 is OPAQUE: empty iv set (NOT {i, k}).
    assert_eq!(
        map.get(&x),
        Some(&vec![BTreeSet::new()]),
        "TASK-0373 AC#2: gather outer array `x` dim 0 must be OPAQUE \
         (empty iv set), NOT attributed the inner ivs {{i, k}} — \
         otherwise x would be wrongly i-banded instead of whole-array.",
    );
    // col_idx stays iv-affine: dim 0 = {i}, dim 1 = {k}.
    assert_eq!(
        map.get(&col_idx),
        Some(&vec![iv_set(&[i_iv]), iv_set(&[k_iv])]),
        "TASK-0373: the index array `col_idx[i][k]` is iv-affine; its \
         dims keep {{i}}, {{k}} so it i-bands like `val`.",
    );

    // End-to-end: x with an opaque dim 0 ⇒ whole-array broadcast.
    let worker = WorkerId(0);
    let partition_ranges = make_partition_ranges(&[(i_iv, &[(worker, 0..2)])]);
    let x_bounds = compute_partition_bounds_with_dim_prefix(x, &map, &partition_ranges, worker);
    assert_eq!(
        x_bounds,
        Some(Vec::new()),
        "TASK-0373 AC#2: x's opaque dim 0 is a hole at dim 0 ⇒ empty \
         prefix ⇒ Some(empty) ⇒ whole-array broadcast.",
    );
}

/// AC#1 stickiness: a dim observed data-dependent on one access
/// stays OPAQUE even when a SIBLING affine access on the same
/// symbol/dim is later observed — the whole-array broadcast must
/// still serve the gather access. (Defensive: no shipped program
/// mixes affine + gather on the same symbol's same dim, but the
/// soundness contract requires stickiness.)
#[test]
fn task0373_opaque_dim_is_sticky_across_affine_sibling_access() {
    let i_iv = IterVar(1);
    let k_iv = IterVar(2);
    let x = DataId(10);
    let mut name_iter_vars: BTreeMap<String, IterVar> = BTreeMap::new();
    name_iter_vars.insert("i".to_string(), i_iv);
    name_iter_vars.insert("k".to_string(), k_iv);

    // First access: gather x[col_idx[i][k]] ⇒ x dim 0 OPAQUE.
    let gather_access = DataAccess {
        data: x,
        indices: vec![IrExpr::DataRef(crate::algo::ir::IndexedRef {
            name: "col_idx".to_string(),
            indices: vec![
                IrExpr::Ident("i".to_string()),
                IrExpr::Ident("k".to_string()),
            ],
        })],
    };
    // Second access on the SAME symbol/dim: affine x[i]. Must NOT
    // un-opaque dim 0.
    let affine_access = DataAccess {
        data: x,
        indices: vec![IrExpr::Ident("i".to_string())],
    };
    let node = op_node_with_accesses(vec![gather_access, affine_access]);

    let map = collect_data_dim_iv_map(&node, &name_iter_vars);
    assert_eq!(
        map.get(&x),
        Some(&vec![BTreeSet::new()]),
        "TASK-0373 stickiness: once x dim 0 is OPAQUE (gather), a \
         sibling affine x[i] access must NOT re-attribute {{i}} — \
         whole-array broadcast must still serve the gather.",
    );
}

/// AC#1 stickiness, REVERSED order (review P3.2): the affine access
/// is observed FIRST (x[i] ⇒ dim 0 = {i}), then the gather
/// x[col_idx[i][k]] is observed. The gather must CLEAR the
/// already-collected {i} and mark the dim opaque — this is the
/// `entry[dim].clear()` arm in `record_access_per_dim`, which the
/// gather-first test does not exercise (there the set is already
/// empty). Removing the `.clear()` would leave dim 0 = {i} and
/// wrongly i-band x, so this test BITES that guard.
#[test]
fn task0373_opaque_dim_is_sticky_when_affine_observed_first() {
    let i_iv = IterVar(1);
    let k_iv = IterVar(2);
    let x = DataId(10);
    let mut name_iter_vars: BTreeMap<String, IterVar> = BTreeMap::new();
    name_iter_vars.insert("i".to_string(), i_iv);
    name_iter_vars.insert("k".to_string(), k_iv);

    // First access: affine x[i] ⇒ dim 0 transiently = {i}.
    let affine_access = DataAccess {
        data: x,
        indices: vec![IrExpr::Ident("i".to_string())],
    };
    // Second access: gather x[col_idx[i][k]] ⇒ must CLEAR {i} and
    // mark dim 0 opaque.
    let gather_access = DataAccess {
        data: x,
        indices: vec![IrExpr::DataRef(crate::algo::ir::IndexedRef {
            name: "col_idx".to_string(),
            indices: vec![
                IrExpr::Ident("i".to_string()),
                IrExpr::Ident("k".to_string()),
            ],
        })],
    };
    let node = op_node_with_accesses(vec![affine_access, gather_access]);

    let map = collect_data_dim_iv_map(&node, &name_iter_vars);
    assert_eq!(
        map.get(&x),
        Some(&vec![BTreeSet::new()]),
        "TASK-0373 stickiness (affine-first): a later gather access \
         must CLEAR the transiently-collected {{i}} and mark dim 0 \
         opaque — otherwise x is wrongly i-banded.",
    );
}

// ----------------------------------------------------------------
// TASK-0341.02.02.01.{02,03} cycle 213: cumulative-array band tile +
// w2w hoist (16-jacobi/distributed).
// ----------------------------------------------------------------

/// `cumulative_band_bounds` for the 16-jacobi `field[5][8][8]` ×
/// `partition=rows(y)` shape from w1 (write band 1..3): the tile must
/// be `[(t, 0..5 FULL), (y, 1..3 BAND), (x, 0..8 FULL)]` — the
/// SENDER write band, NOT halo-expanded, NOT whole-array.
#[test]
fn task034102_cumulative_band_bounds_field_write_band() {
    let t_iv = IterVar(2);
    let y_iv = IterVar(4);
    let x_iv = IterVar(3);
    let field = DataId(0);
    let w1 = WorkerId(1);
    // field indexed dim0=t, dim1=y, dim2=x.
    let mut map: BTreeMap<DataId, Vec<BTreeSet<IterVar>>> = BTreeMap::new();
    map.insert(
        field,
        vec![iv_set(&[t_iv]), iv_set(&[y_iv]), iv_set(&[x_iv])],
    );
    let partition_ranges =
        make_partition_ranges(&[(y_iv, &[(WorkerId(1), 1..3), (WorkerId(2), 3..5)])]);
    let dims = vec![5i64, 8, 8];

    let got = cumulative_band_bounds(field, w1, &dims, &map, &partition_ranges)
        .expect("cumulative band tile must be constructible");
    assert_eq!(
        got,
        vec![(t_iv, 0..5), (y_iv, 1..3), (x_iv, 0..8)],
        "cumulative write-band tile must be FULL on t/x and BANDED (1..3) on \
         the partition axis y, keyed on the SENDER (w1) write band; got {got:?}"
    );
}

/// TASK-0366 cycle-214 (cycle-213 architect P3), CASE (A) — the
/// genuine xN-risk shape. The formerly-silent whole-array fallback in
/// `rewrite_cumulative_band_tiles` is now a fail-loud typed error WHEN
/// A PARTITION IS ACTIVE but no band could be derived. Drive a single
/// cumulative `Xfer` whose `cumulative_band_bounds` returns `None`
/// WITH `partition_ranges` NON-EMPTY, and assert the new
/// `CumulativeWholeArrayFallback` variant fires.
///
/// Forcing the `None` path: `data_dim_iv_map[data]` has the right dim
/// count (so the `per_dim.len() != dims.len()` early-`None` is NOT the
/// cause), but the (decoy) partitioned iv does NOT index this data —
/// so no dim resolves to a band, `saw_band` stays `false`, and
/// `cumulative_band_bounds` returns `None`. The data is BOTH in
/// `cumulative_data` AND `data_dims` (the two preconditions to reach
/// the inner branch), and `partition_ranges` is non-empty (a real
/// partition is active → the array is replicated-across-workers → the
/// whole-array tile WOULD xN-double-count). This is the provably-dead-
/// today branch; the test pins that, were a future partitioned-
/// cumulative schedule to reach it, the compiler rejects rather than
/// emitting an xN-double-counted whole-array tile.
#[test]
fn task0366_partitioned_cumulative_none_band_raises_fail_loud_error() {
    let t_iv = IterVar(2);
    let y_iv = IterVar(4);
    let x_iv = IterVar(3);
    let field = DataId(0);
    let src = WorkerId(1);
    let cumulative: BTreeSet<DataId> = [field].into_iter().collect();

    // data_dim_iv_map: 3 dims, matching dims.len() below — so the
    // dim-count early-None does NOT fire. But the partition_ranges
    // below cover NONE of {t,y,x}, so no dim resolves to a band.
    let mut data_dim_iv_map: BTreeMap<DataId, Vec<BTreeSet<IterVar>>> = BTreeMap::new();
    data_dim_iv_map.insert(
        field,
        vec![iv_set(&[t_iv]), iv_set(&[y_iv]), iv_set(&[x_iv])],
    );
    let mut data_dims: BTreeMap<DataId, Vec<i64>> = BTreeMap::new();
    data_dims.insert(field, vec![5i64, 8, 8]);

    // A partition on an iv that does NOT index `field` (decoy) —
    // ensures the helper iterates the partitioned-iv filter but finds
    // no covering iv per dim, so saw_band == false → None. Crucially
    // `partition_ranges` is NON-EMPTY, so this is CASE (A): the array
    // is replicated across the (decoy_iv) partition workers and the
    // whole-array tile would xN-double-count.
    let decoy_iv = IterVar(99);
    let partition_ranges =
        make_partition_ranges(&[(decoy_iv, &[(WorkerId(1), 0..4), (WorkerId(2), 4..8)])]);

    let policy = TransferPolicy {
        synchronous: true,
        buffer: 1,
        notify: NotifyMode::Default,
    };
    let xfer = ACFGNode::Xfer(XferPlaceholder {
        role: XferRole::Push,
        src,
        dst: WorkerId(0),
        data: field,
        // Incoming whole-array tile — exactly the tile that would be
        // silently kept (xN risk) before TASK-0366.
        tile: IterTile::new(vec![(t_iv, 0..5), (y_iv, 0..8), (x_iv, 0..8)]),
        seq: SeqTag(0),
        policy,
    });

    let result = rewrite_cumulative_band_tiles(
        xfer,
        &cumulative,
        &partition_ranges,
        &data_dim_iv_map,
        &data_dims,
    );

    match result {
        Err(TransferInjectError::CumulativeWholeArrayFallback {
            data,
            src: err_src,
            message,
        }) => {
            assert_eq!(
                data, field,
                "the error must name the offending cumulative DataId (field)"
            );
            assert_eq!(
                err_src, src,
                "the error must carry the transfer's src worker (band owner)"
            );
            assert!(
                message.contains("TASK-0366"),
                "message must forward-link TASK-0366; got: {message}"
            );
            assert!(
                message.contains("xN"),
                "message must name the xN double-count risk; got: {message}"
            );
        }
        other => panic!(
            "TASK-0366 case A: a partitioned cumulative Xfer with no derivable write band \
             MUST raise CumulativeWholeArrayFallback, not silently keep the whole-array \
             tile; got: {other:?}"
        ),
    }
}

/// TASK-0366 cycle-214, CASE (B) — the UNPARTITIONED cumulative
/// symbol (11-game-of-life/pipelined `grid`). Same `None` from
/// `cumulative_band_bounds`, but `partition_ranges` is EMPTY: there
/// is no partition to double-count against, so the whole-array tile
/// is CORRECT and the pass must keep it SILENTLY (NOT raise the
/// error). This pins the A/B discriminator — without it, the
/// `!partition_ranges.is_empty()` guard would silently regress to
/// rejecting the game-of-life shape (which the e2e gate caught when
/// the first TASK-0366 draft made the branch unconditional).
#[test]
fn task0366_unpartitioned_cumulative_keeps_whole_array_no_error() {
    let t_iv = IterVar(2);
    let i_iv = IterVar(3);
    let grid = DataId(0);
    let compute = WorkerId(1);
    let cumulative: BTreeSet<DataId> = [grid].into_iter().collect();

    // grid[ITERS+1][N] indexed dim0=t, dim1=i — both ivs present, so
    // the dim-count early-None does NOT fire, but with NO partition
    // active `cumulative_band_bounds` still returns None (saw_band
    // false).
    let mut data_dim_iv_map: BTreeMap<DataId, Vec<BTreeSet<IterVar>>> = BTreeMap::new();
    data_dim_iv_map.insert(grid, vec![iv_set(&[t_iv]), iv_set(&[i_iv])]);
    let mut data_dims: BTreeMap<DataId, Vec<i64>> = BTreeMap::new();
    data_dims.insert(grid, vec![9i64, 32]);

    // EMPTY partition_ranges — the game-of-life/pipelined shape.
    let partition_ranges: BTreeMap<IterVar, BTreeMap<WorkerId, std::ops::Range<i64>>> =
        BTreeMap::new();

    let policy = TransferPolicy {
        synchronous: false,
        buffer: 2,
        notify: NotifyMode::Default,
    };
    let whole_array_tile = IterTile::new(vec![(t_iv, 0..9), (i_iv, 0..32)]);
    let xfer = ACFGNode::Xfer(XferPlaceholder {
        role: XferRole::Push,
        src: compute,
        dst: WorkerId(0),
        data: grid,
        tile: whole_array_tile.clone(),
        seq: SeqTag(0),
        policy,
    });

    let result = rewrite_cumulative_band_tiles(
        xfer,
        &cumulative,
        &partition_ranges,
        &data_dim_iv_map,
        &data_dims,
    );

    match result {
        Ok(ACFGNode::Xfer(x)) => {
            assert_eq!(
                x.tile, whole_array_tile,
                "TASK-0366 case B: an UNPARTITIONED cumulative symbol must keep its \
                 whole-array tile unchanged (no partition to xN-double-count against); \
                 got tile {:?}",
                x.tile
            );
        }
        other => panic!(
            "TASK-0366 case B: an unpartitioned cumulative Xfer must be kept as a \
             whole-array transfer (Ok), NOT rejected; got: {other:?}"
        ),
    }
}

/// TASK-0366 cycle-214 architect P3 fold-back — confirm the
/// fail-loud error PROPAGATES through the recursive `Sequence` /
/// `Repeat` arms (the `?`-threading), not just from a bare top-level
/// `Xfer` leaf. Case A and B drive the function with a leaf node; if
/// a future edit broke the `collect::<Result<Vec<_>, _>>()?` in the
/// `Sequence` arm or the boxed `?` in the `Repeat` arm, those two
/// tests would still pass and only the e2e gate would (indirectly)
/// catch it. Here the Case-A Xfer is buried inside
/// `Sequence([ Repeat(t) { Sequence([ Xfer ]) } ])`, so an `Err`
/// reaching the caller proves both recursive arms re-raise.
#[test]
fn task0366_fail_loud_error_propagates_through_nested_sequence_and_repeat() {
    let t_iv = IterVar(2);
    let y_iv = IterVar(4);
    let x_iv = IterVar(3);
    let field = DataId(0);
    let src = WorkerId(1);
    let cumulative: BTreeSet<DataId> = [field].into_iter().collect();

    let mut data_dim_iv_map: BTreeMap<DataId, Vec<BTreeSet<IterVar>>> = BTreeMap::new();
    data_dim_iv_map.insert(
        field,
        vec![iv_set(&[t_iv]), iv_set(&[y_iv]), iv_set(&[x_iv])],
    );
    let mut data_dims: BTreeMap<DataId, Vec<i64>> = BTreeMap::new();
    data_dims.insert(field, vec![5i64, 8, 8]);

    // Decoy partition (non-empty → Case A) on an iv that does NOT
    // index `field`, so `cumulative_band_bounds` returns None.
    let decoy_iv = IterVar(99);
    let partition_ranges =
        make_partition_ranges(&[(decoy_iv, &[(WorkerId(1), 0..4), (WorkerId(2), 4..8)])]);

    let policy = TransferPolicy {
        synchronous: true,
        buffer: 1,
        notify: NotifyMode::Default,
    };
    let xfer = ACFGNode::Xfer(XferPlaceholder {
        role: XferRole::Push,
        src,
        dst: WorkerId(0),
        data: field,
        tile: IterTile::new(vec![(t_iv, 0..5), (y_iv, 0..8), (x_iv, 0..8)]),
        seq: SeqTag(0),
        policy,
    });
    // Bury the offending Xfer two levels deep: Sequence -> Repeat ->
    // Sequence -> Xfer. Reaching it exercises BOTH the Sequence
    // `collect::<Result>?` and the Repeat boxed `?`.
    let nested = ACFGNode::Sequence(vec![ACFGNode::Repeat {
        iter_var: t_iv,
        range: 0..5,
        body: Box::new(ACFGNode::Sequence(vec![xfer])),
        block_tag: None,
    }]);

    let result = rewrite_cumulative_band_tiles(
        nested,
        &cumulative,
        &partition_ranges,
        &data_dim_iv_map,
        &data_dims,
    );

    assert!(
        matches!(
            result,
            Err(TransferInjectError::CumulativeWholeArrayFallback { data, .. }) if data == field
        ),
        "TASK-0366 P3: the fail-loud error must propagate out of a nested \
         Sequence/Repeat (the `?`-threading), not be swallowed; got: {result:?}"
    );
}

/// `strip_cumulative_xfers` lifts the cumulative-data Xfers out of a
/// nested loop subtree and leaves the rest intact; `hoist_cumulative_
/// w2w_to_repeat_body` then re-places them AFTER the partition Repeat
/// in SEND-then-RECV order. Synthetic minimal shape:
///   Repeat(t) { [ Repeat(y=partition) { [ Wait(field), Op, Push(field) ] } ] }
/// →
///   Repeat(t) { [ Repeat(y) { [ Op ] }, Push(field), Wait(field) ] }
#[test]
fn task034102_hoist_w2w_send_then_recv_after_partition_repeat() {
    let field = DataId(0);
    let t_iv = IterVar(2);
    let y_iv = IterVar(4);
    let cumulative: BTreeSet<DataId> = [field].into_iter().collect();
    let partition_ranges =
        make_partition_ranges(&[(y_iv, &[(WorkerId(1), 1..3), (WorkerId(2), 3..5)])]);

    let policy = TransferPolicy {
        synchronous: true,
        buffer: 1,
        notify: NotifyMode::Default,
    };
    let wait = ACFGNode::Xfer(XferPlaceholder {
        role: XferRole::Wait,
        src: WorkerId(2),
        dst: WorkerId(1),
        data: field,
        tile: IterTile::empty(),
        seq: SeqTag(3),
        policy,
    });
    let push = ACFGNode::Xfer(XferPlaceholder {
        role: XferRole::Push,
        src: WorkerId(1),
        dst: WorkerId(2),
        data: field,
        tile: IterTile::empty(),
        seq: SeqTag(0),
        policy,
    });
    // A non-Xfer leaf standing in for the band-compute Operation.
    let compute = ACFGNode::Sync(crate::acfg::SyncPlaceholder::default());
    let inner_y = ACFGNode::Repeat {
        iter_var: y_iv,
        range: 1..3,
        body: Box::new(ACFGNode::Sequence(vec![
            wait.clone(),
            compute.clone(),
            push.clone(),
        ])),
        block_tag: None,
    };
    let for_t = ACFGNode::Repeat {
        iter_var: t_iv,
        range: 0..5,
        body: Box::new(ACFGNode::Sequence(vec![inner_y])),
        block_tag: None,
    };
    let root = ACFGNode::Sequence(vec![for_t]);

    let hoisted = hoist_cumulative_w2w_to_repeat_body(root, &cumulative, &partition_ranges);

    // Expected: Repeat(t) { Sequence[ Repeat(y){ Sequence[ compute ] },
    // Push, Wait ] }.
    let ACFGNode::Sequence(top) = hoisted else {
        panic!("expected top Sequence")
    };
    let ACFGNode::Repeat { body, .. } = &top[0] else {
        panic!("expected for_t Repeat")
    };
    let ACFGNode::Sequence(t_body) = body.as_ref() else {
        panic!("expected for_t body Sequence")
    };
    // t_body = [ Repeat(y), Push, Wait ] — send (Push) BEFORE recv (Wait).
    assert_eq!(
        t_body.len(),
        3,
        "for_t body should be [Repeat(y), Push, Wait]; got {t_body:?}"
    );
    assert!(
        matches!(&t_body[0], ACFGNode::Repeat { iter_var, .. } if *iter_var == y_iv),
        "first child must be the partition Repeat (compute stays); got {:?}",
        t_body[0]
    );
    assert!(
        matches!(&t_body[1], ACFGNode::Xfer(x) if x.role == XferRole::Push),
        "SEND-then-recv: the Push must come BEFORE the Wait; got {:?}",
        t_body[1]
    );
    assert!(
        matches!(&t_body[2], ACFGNode::Xfer(x) if x.role == XferRole::Wait),
        "the Wait must come AFTER the Push; got {:?}",
        t_body[2]
    );
    // The partition Repeat's body must no longer contain the field Xfers.
    let ACFGNode::Repeat { body: y_body, .. } = &t_body[0] else {
        unreachable!()
    };
    let mut leftover: Vec<XferPlaceholder> = Vec::new();
    let _ = strip_cumulative_xfers((**y_body).clone(), &cumulative, &mut leftover);
    assert!(
        leftover.is_empty(),
        "the partition Repeat body must have NO cumulative Xfers left after the \
         hoist; got {leftover:?}"
    );
}

/// A tree with an empty cumulative set is left byte-identical by the
/// hoist pass (the partition-guarded no-op path taken by every example
/// that ships no partitioned cumulative array — i.e. all but
/// 16-jacobi/distributed today).
#[test]
fn task034102_hoist_noop_when_no_cumulative_data() {
    let field = DataId(0);
    let y_iv = IterVar(4);
    let partition_ranges = make_partition_ranges(&[(y_iv, &[(WorkerId(1), 1..3)])]);
    let policy = TransferPolicy {
        synchronous: true,
        buffer: 1,
        notify: NotifyMode::Default,
    };
    let inner = ACFGNode::Repeat {
        iter_var: y_iv,
        range: 1..3,
        body: Box::new(ACFGNode::Sequence(vec![ACFGNode::Xfer(XferPlaceholder {
            role: XferRole::Push,
            src: WorkerId(1),
            dst: WorkerId(2),
            data: field,
            tile: IterTile::empty(),
            seq: SeqTag(0),
            policy,
        })])),
        block_tag: None,
    };
    let root = ACFGNode::Sequence(vec![inner]);
    let empty: BTreeSet<DataId> = BTreeSet::new();
    let got = hoist_cumulative_w2w_to_repeat_body(root.clone(), &empty, &partition_ranges);
    assert_eq!(
        got, root,
        "empty cumulative set MUST leave the tree unchanged"
    );
}
