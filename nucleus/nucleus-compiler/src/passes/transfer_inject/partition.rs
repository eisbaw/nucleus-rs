use super::*;

// --------------------------------------------------------------------
// TASK-0117 per-worker tile finalisation
// --------------------------------------------------------------------

/// Walk the ACFG and rewrite every `Xfer`'s `tile` to its
/// fan-out compute worker's partition slice.
///
/// "Compute worker" rule:
/// - If `src` appears in `partition_ranges` for any iter-var:
///   compute_worker = src (the N:1 gather direction, e.g. output).
/// - Else if `dst` appears: compute_worker = dst (the 1:N broadcast
///   direction, e.g. input).
/// - Else: no partition involvement — leave the tile unchanged
///   (the 1:1 host↔single-worker shape from examples 01..07).
///
/// The new tile is built by iterating the partitioned iter-vars in
/// NEST ORDER (outer-to-inner) and appending
/// `(iv, partition_ranges[iv][compute_worker])` for each iv where the
/// compute worker has a partition entry. The nest order itself is
/// derived from the ACFG topology via a single DFS pre-order pass over
/// the `Repeat` nodes — outer Repeats are visited before their
/// children — restricted to iter-vars that have a `partition_ranges`
/// entry. The resulting `bounds` is naturally OUTER-to-INNER, the
/// convention load-bearing for downstream passes
/// (`annotate_pipeline_depth_for_seq` walks `bounds.iter().rev()` for
/// "innermost wins").
///
/// TASK-0224 replaced an earlier per-Xfer `for (iv, ...) in
/// partition_ranges` loop that iterated in BTreeMap key-ascending =
/// IterVar-id ascending order. That coincided with nest order for
/// schedules where outer-loop iter-vars happened to have lower ids
/// (every in-tree schedule today) but is not guaranteed by the
/// `IterTile::bounds` convention. Pinned by
/// `rewrite_partition_tiles_bounds_in_nest_order_not_itervar_id_order`
/// and `rewrite_partition_tiles_three_level_nest_order` (synthetic
/// fixtures with non-monotonic IterVar ids).
///
/// Why we derive nest order from the ACFG rather than from the live
/// "enclosing-Repeat stack" at the Xfer's current position: by the
/// time this pass runs, `hoist_invariant_waits` has already moved
/// loop-invariant whole-symbol Waits OUT of their birth-position
/// Repeats (TASK-0151). For a fan-out broadcast like `host -> {w1..w4}`
/// over a partitioned `for n`, the Wait now sits at top-level, with an
/// empty live-stack — but we still want the per-worker partition slice
/// of `for n` recorded in its tile (the test
/// `transfer_fanout_composes_with_partition_sidecar` pins exactly
/// that). The Repeat node still exists in the ACFG, just at a sibling
/// position; a DFS pre-order over `Repeat::iter_var` captures the
/// program-wide nest order regardless of where any one Xfer happens
/// to have been hoisted to.
///
/// Determinism: DFS pre-order is a pure function of the ACFG; the
/// resulting `partition_axis_order` is the same for the same input.
///
/// We do this AFTER `hoist_invariant_waits` + `splice_pushes_global`
/// rather than at `build_waits_for_op` time because the hoist
/// rewrites the tile to the post-hoist enclosing tile (TASK-0151), so
/// a tile set at construction time would be clobbered for any Wait
/// hoisted out of the partitioned loop body. Setting the tile here
/// is the single sink for the contract.
pub(super) fn rewrite_partition_tiles(
    node: ACFGNode,
    partition_ranges: &BTreeMap<IterVar, BTreeMap<WorkerId, std::ops::Range<i64>>>,
    data_dim_iv_map: &BTreeMap<DataId, Vec<BTreeSet<IterVar>>>,
) -> ACFGNode {
    if partition_ranges.is_empty() {
        return node;
    }
    // Derive the program-wide nest order of partitioned iter-vars via
    // a DFS pre-order over `Repeat::iter_var`, filtered to those in
    // `partition_ranges`. Outer Repeats appear before their children,
    // so the resulting Vec is OUTER-to-INNER. The same iter-var
    // appearing twice (e.g. via `block_transform` strip-mining that
    // reuses an IterVar across a full/partial split) is recorded only
    // on its first visit — the partition slice is keyed per iter-var,
    // and a second copy at deeper nest would double the bounds entry
    // for the same logical axis.
    //
    // `partition_axis_order` is the FALL-BACK iteration order used when
    // a data symbol has no observed indexed accesses (TASK-0301
    // additive contract → preserves pre-TASK-0301 behaviour for
    // synthetic test fixtures using `DataflowEdge::new`). For data
    // symbols WITH observed accesses, `data_dim_iv_map` carries the
    // per-dim info needed for the TASK-0302 dim-prefix logic, which
    // emits bounds in *data-dim* order (matching `wait_slice`'s
    // axis-mapping convention).
    let mut partition_axis_order: Vec<IterVar> = Vec::new();
    collect_partitioned_iter_var_nest_order(&node, partition_ranges, &mut partition_axis_order);
    rewrite_partition_tiles_inner(
        node,
        partition_ranges,
        &partition_axis_order,
        data_dim_iv_map,
    )
}

/// DFS pre-order walk of the ACFG recording each `Repeat::iter_var`
/// that has a `partition_ranges` entry, OUTER-to-INNER. Each iter-var
/// is recorded at most once even if a `Repeat` for the same `IterVar`
/// appears multiple times (e.g. strip-mined full/partial split sharing
/// one `IterVar`); first-occurrence wins, which is the outermost
/// occurrence under DFS pre-order.
pub(super) fn collect_partitioned_iter_var_nest_order(
    node: &ACFGNode,
    partition_ranges: &BTreeMap<IterVar, BTreeMap<WorkerId, std::ops::Range<i64>>>,
    out: &mut Vec<IterVar>,
) {
    match node {
        ACFGNode::Repeat { iter_var, body, .. } => {
            if partition_ranges.contains_key(iter_var) && !out.contains(iter_var) {
                out.push(*iter_var);
            }
            collect_partitioned_iter_var_nest_order(body, partition_ranges, out);
        }
        ACFGNode::Sequence(children) => {
            for c in children {
                collect_partitioned_iter_var_nest_order(c, partition_ranges, out);
            }
        }
        ACFGNode::Operation(_) | ACFGNode::Sync(_) | ACFGNode::Xfer(_) => {}
    }
}

/// Recursive worker for [`rewrite_partition_tiles`].
///
/// `partition_axis_order` is the OUTER-to-INNER nest-order vec of
/// partitioned iter-vars precomputed by the caller. As of TASK-0302 it
/// is consulted only on the *fall-back* path (data symbols with no
/// observed indexed accesses — synthetic fixtures using
/// `DataflowEdge::new`, OR bare-aggregate-only data symbols). The
/// canonical path keys off `data_dim_iv_map` via
/// [`compute_partition_bounds_with_dim_prefix`], which emits bounds in
/// data-dim order (matching `wait_slice`'s
/// `tile.bounds[i] ↔ data.dim[i]` convention).
pub(super) fn rewrite_partition_tiles_inner(
    node: ACFGNode,
    partition_ranges: &BTreeMap<IterVar, BTreeMap<WorkerId, std::ops::Range<i64>>>,
    partition_axis_order: &[IterVar],
    data_dim_iv_map: &BTreeMap<DataId, Vec<BTreeSet<IterVar>>>,
) -> ACFGNode {
    match node {
        ACFGNode::Xfer(mut x) => {
            // Pick the compute worker per the rule above.
            let compute_worker = if partition_ranges.values().any(|m| m.contains_key(&x.src)) {
                Some(x.src)
            } else if partition_ranges.values().any(|m| m.contains_key(&x.dst)) {
                Some(x.dst)
            } else {
                None
            };
            if let Some(w) = compute_worker {
                // TASK-0302: prefer the per-dim contiguous-prefix
                // filter (which subsumes TASK-0301's per-symbol-union
                // filter and additionally handles the sparse-coverage
                // case where partition axes do NOT form a prefix of
                // the data's dims). Returns:
                //   - `Some(bounds)` — emit in DATA-DIM order. Empty
                //     vec means "drop to whole-array broadcast"
                //     (sparse coverage OR ambiguous multi-iv-per-dim).
                //   - `None` — no observed indexed accesses on this
                //     data; fall back to the pre-TASK-0301 nest-order
                //     iteration that preserves the synthetic-fixture
                //     contract (DataflowEdge::new constructs accesses
                //     with empty indices).
                //
                // Silent-sibling audit (architect P3.3, cycle 118;
                // updated TASK-0306 cycle 133; cycle-137 doc-lie
                // audit listing-correction; cycle-137 architect P1
                // fold-back adds `build_waits_for_op`, fixes line
                // cites for `extend_xfer_tiles_inner` and
                // `inject_halo_strip_xfers`; TASK-0319 cycle 146
                // migrates absolute line citations to function-name
                // anchors as primary indices — line numbers are
                // ADVISORY-ONLY): every other site that mutates
                // `x.tile` / `w.tile` OR constructs an `IterTile`
                // carrying the enclosing-tile assumption either:
                //
                // - **Builds tile from enclosing-loop stack** — three
                //   production sites, all using
                //   `IterTile::new(enclosing_tile.to_vec())`:
                //     1. `inject_in_sequence` — hoisted-Wait tile
                //        rewrite at sequence boundary, called via
                //        `inject_in_node_with_tile`'s dispatch.
                //     2. `hoist_invariant_waits` — Wait tile rewrite
                //        when an invariant-Wait is hoisted across an
                //        enclosing sequence (separate post-pass, NOT
                //        in the `inject_in_node_with_tile` family).
                //     3. `build_waits_for_op` — INITIAL
                //        `XferPlaceholder` tile on every cross-
                //        worker Wait the cartesian fan-out emits.
                //        This `rewrite_partition_tiles_inner` arm
                //        later overwrites it with `bounds`.
                //   None of the three references `partition_ranges`
                //   or `data_dim_iv_map` in its own scope, so the
                //   axis-mapping assumption cannot leak in via them.
                //   (Since the TASK-0340.13 split the three sites are
                //   in sibling submodules, NOT this file:
                //   `inject_in_sequence` and `hoist_invariant_waits` in
                //   `sequence`, `build_waits_for_op` in `ordering`.)
                //   Grep witnesses (must remain consistent):
                //   - `grep -rnE "IterTile::new\(enclosing_tile\.to_vec\(\)\)"`
                //     over the `transfer_inject/` dir returns 5 hits:
                //     3 PRODUCTION code-sites (identifiable by
                //     `w.tile =` / `tile:` LHS — two in `sequence.rs`,
                //     one in `ordering.rs`) + 1 module-doc citation in
                //     `mod.rs` (the `//!`-prefixed line) + 1
                //     self-reference inside THIS audit listing (the
                //     `//   `-prefixed line). Filter out commentary
                //     lines (`... | grep -vE ":\s*//"`) to count only
                //     the three production sites directly.
                //   - `grep -nE "data_dim_iv_map|partition_ranges"`
                //     restricted to the body of each of those three
                //     functions returns zero hits.
                // - **Extends an already-filtered bounds set** —
                //   `extend_xfer_tiles_for_halo`, with the per-Xfer
                //   mutation inside the worker `extend_xfer_tiles_inner`.
                //   Runs AFTER this pass per the explicit pass-order
                //   call in `inject_transfers`. Iterates the post-
                //   partition `x.tile.bounds` and widens each range
                //   by halo, preserving order.
                // - **Hand-crafts the (outer_iv, inner_iv) pair from
                //   the partition_pairs sidecar** —
                //   `inject_halo_strip_xfers` constructs fresh tiles
                //   at FOUR cardinal-direction `emit_pair` sites
                //   AFTER `order_halo_strip_bounds_by_data_dim`
                //   returns the data-dim-ordered bounds; axis-
                //   correct since cycle 133 by construction. Grep
                //   witness: `grep -nE "emit_pair\(neighbour,"`
                //   (trailing comma excludes this self-reference)
                //   returns exactly four production sites (N/S/W/E
                //   neighbours), all inside `inject_halo_strip_xfers`.
                //
                // A future N-dim partition pass that constructs tile
                // bounds MUST consult `data_dim_iv_map` to avoid
                // re-importing the axis-mapping assumption — using
                // either:
                //   - `compute_partition_bounds_with_dim_prefix` for
                //     REGULAR-XFER TILE REWRITE (the path this match
                //     arm is on), where partitioned bounds replace
                //     the existing source tile under wait_slice's
                //     dim convention; or
                //   - `order_halo_strip_bounds_by_data_dim` for
                //     HALO-STRIP PUSH/WAIT SYNTHESIS (the
                //     `inject_halo_strip_xfers` path), where a
                //     freshly-constructed `(outer_iv, inner_iv)`
                //     tuple must be re-ordered into data-dim order
                //     before emit.
                let bounds = match compute_partition_bounds_with_dim_prefix(
                    x.data,
                    data_dim_iv_map,
                    partition_ranges,
                    w,
                ) {
                    Some(b) => b,
                    None => {
                        // Pre-TASK-0301 fall-back: iterate the
                        // partition_axis_order (nest order) and append
                        // every partitioned axis the worker has an
                        // entry for. Used only when the data symbol
                        // has no observed indexed accesses (synthetic
                        // fixtures + bare-aggregate-only data).
                        //
                        // TASK-0317: emit a `NUC_TRACE`-gated diagnostic
                        // on the fall-back. Mirrors the cycle-134
                        // (TASK-0315) defence on
                        // `order_halo_strip_bounds_by_data_dim`. The
                        // TASK-0301 axis-mapping defence (data-dim-aware
                        // emit) is BYPASSED on this call; trace makes
                        // the bypass observable so a future regression
                        // masked by synthetic-fixture coverage is not
                        // silent. Production callers always observe
                        // accesses; this trace fires only on synthetic
                        // / bare-aggregate-only data paths.
                        // cycle-135 architect P2-1 fold-back: mirror
                        // cycle-134's `entry = absent | Some(empty)`
                        // disambiguator. `compute_partition_bounds_with_
                        // dim_prefix` returns None on exactly two paths
                        // (dim_iv_map.get is None, or per_dim.is_empty())
                        // so the 2-arm match is exhaustive at the trace
                        // site by the helper's contract — parallel to
                        // cycle-134's identical disambiguator shape.
                        crate::nuc_trace!(
                            "transfer_inject::rewrite_partition_tiles_inner: fall-back to \
                             partition_axis_order nest-order emit (data={data:?}, worker={w:?}); \
                             compute_partition_bounds_with_dim_prefix returned None — \
                             data_dim_iv_map entry is {entry} — \
                             TASK-0301 axis-mapping defence BYPASSED for this call \
                             (expected only on synthetic fixtures via DataflowEdge::new or \
                             bare-aggregate-only data symbols)",
                            data = x.data,
                            entry = match data_dim_iv_map.get(&x.data) {
                                None => "absent",
                                Some(_) => "Some(empty)",
                            },
                        );
                        let mut tmp: Vec<(IterVar, std::ops::Range<i64>)> = Vec::new();
                        for iv in partition_axis_order {
                            if let Some(per_worker) = partition_ranges.get(iv) {
                                if let Some(range) = per_worker.get(&w) {
                                    tmp.push((*iv, range.clone()));
                                }
                            }
                        }
                        tmp
                    }
                };
                if !bounds.is_empty() {
                    x.tile = IterTile::new(bounds);
                }
            }
            ACFGNode::Xfer(x)
        }
        ACFGNode::Sequence(children) => ACFGNode::Sequence(
            children
                .into_iter()
                .map(|c| {
                    rewrite_partition_tiles_inner(
                        c,
                        partition_ranges,
                        partition_axis_order,
                        data_dim_iv_map,
                    )
                })
                .collect(),
        ),
        ACFGNode::Repeat {
            iter_var,
            range,
            body,
            block_tag,
        } => ACFGNode::Repeat {
            iter_var,
            range,
            body: Box::new(rewrite_partition_tiles_inner(
                *body,
                partition_ranges,
                partition_axis_order,
                data_dim_iv_map,
            )),
            block_tag,
        },
        leaf @ (ACFGNode::Operation(_) | ACFGNode::Sync(_)) => leaf,
    }
}

// --------------------------------------------------------------------
// TASK-0301 / TASK-0302: data → per-dim iter-vars indexing
// (axis-mapping filter input)
// --------------------------------------------------------------------

/// Build the per-data, per-dim iter-var sets observed across every
/// indexed access on the symbol.
///
/// Returns `BTreeMap<DataId, Vec<BTreeSet<IterVar>>>` where the inner
/// `Vec` is indexed by *data dim position* (the position in
/// [`DataAccess::indices`]) and each set is the union of iter-vars
/// referenced by the index expression at that dim across all observed
/// accesses on the data. The Vec length equals the maximum index-count
/// seen across accesses; ragged accesses (one site indexing fewer dims
/// than another — pathological, not seen in canonical AlgoIR) are
/// tolerated.
///
/// TASK-0302 generalisation of the TASK-0301 per-symbol-union map: the
/// per-dim positions are what
/// [`compute_partition_bounds_with_dim_prefix`] needs to enforce the
/// *contiguous-prefix* invariant that `wait_slice`'s axis-mapping
/// convention rests on. The per-symbol union the older code produced is
/// recoverable as `iter().flatten().copied().collect()` over a data's
/// Vec entry — kept implicit; no caller needs it any more.
///
/// Index-extraction semantics (TASK-0301, OPAQUE-dim refinement
/// TASK-0373):
/// - `IrExpr::Ident(name)` leaves whose `name` resolves through
///   `name_iter_vars` to an `IterVar` contribute that iv to the
///   per-dim set.
/// - A non-`Ident` leaf (`IntLit`, `Neg`/`BinOp` over only consts) records
///   no iv — those axes are partition-invariant.
/// - Arithmetic over an iv (`y - 1`, `k + halo`) still records the iv
///   via recursion on the `BinOp`/`Neg` arms.
/// - A dim whose index expression contains a `DataRef`/`Call` anywhere
///   (a data-dependent GATHER index, e.g. `x[col_idx[i][k]]`) is
///   recorded **OPAQUE**: its per-dim iv set is left EMPTY and the
///   inner ivs (`i`, `k` from `col_idx[i][k]`) are NOT attributed to
///   the outer array's dim. An empty iv set makes
///   [`compute_partition_bounds_with_dim_prefix`] treat the dim as a
///   "hole" → the data falls to whole-array broadcast, which is the
///   only conservatively-sound transfer for a data-dependent read (the
///   worker may load ANY column index at runtime). Opacity is STICKY:
///   once a dim is observed data-dependent on any access, a sibling
///   affine access (`x[i]` elsewhere) does NOT re-populate it — the
///   whole-array broadcast must still serve the gather access. Before
///   TASK-0373 this arm descended into the inner `DataRef` indices and
///   mis-attributed `{i, k}` to the outer array's dim 0, which would
///   have emitted a WRONG i-band slice for `x`.
///
/// Symbols seen only as bare-aggregate accesses (`save_c(c)` style:
/// `indices.is_empty()`) get an entry with an *empty* `Vec` — the same
/// fall-back trigger
/// [`compute_partition_bounds_with_dim_prefix`] treats as "no observed
/// dim info, pre-TASK-0301 contract applies". Symbols not seen on any
/// access at all get no map entry, same fall-back path.
///
/// Structurally independent of `extend_xfer_tiles_for_halo` (which keys
/// off the `halo_widths` sidecar) — the two are only required to be
/// *consistent*, not coupled.
pub(super) fn collect_data_dim_iv_map(
    root: &ACFGNode,
    name_iter_vars: &BTreeMap<String, IterVar>,
) -> BTreeMap<DataId, Vec<BTreeSet<IterVar>>> {
    let mut out: BTreeMap<DataId, Vec<BTreeSet<IterVar>>> = BTreeMap::new();
    // TASK-0373: per-(data, dim) set of dims observed data-dependent
    // (gather). Sticky: an opaque dim stays empty even if a sibling
    // affine access on the same symbol/dim is later observed.
    let mut opaque_dims: BTreeMap<DataId, BTreeSet<usize>> = BTreeMap::new();
    walk_data_dim_iv_map(root, name_iter_vars, &mut out, &mut opaque_dims);
    out
}

pub(super) fn walk_data_dim_iv_map(
    node: &ACFGNode,
    name_iter_vars: &BTreeMap<String, IterVar>,
    out: &mut BTreeMap<DataId, Vec<BTreeSet<IterVar>>>,
    opaque_dims: &mut BTreeMap<DataId, BTreeSet<usize>>,
) {
    match node {
        ACFGNode::Operation(op) => {
            for edge in &op.dataflow.edges {
                for access in &edge.data_in_access {
                    record_access_per_dim(access, name_iter_vars, out, opaque_dims);
                }
                if let Some(access) = &edge.data_out_access {
                    record_access_per_dim(access, name_iter_vars, out, opaque_dims);
                }
            }
        }
        ACFGNode::Sequence(children) => {
            for c in children {
                walk_data_dim_iv_map(c, name_iter_vars, out, opaque_dims);
            }
        }
        ACFGNode::Repeat { body, .. } => {
            walk_data_dim_iv_map(body, name_iter_vars, out, opaque_dims);
        }
        ACFGNode::Sync(_) | ACFGNode::Xfer(_) => {}
    }
}

pub(super) fn record_access_per_dim(
    access: &DataAccess,
    name_iter_vars: &BTreeMap<String, IterVar>,
    out: &mut BTreeMap<DataId, Vec<BTreeSet<IterVar>>>,
    opaque_dims: &mut BTreeMap<DataId, BTreeSet<usize>>,
) {
    let entry = out.entry(access.data).or_default();
    if entry.len() < access.indices.len() {
        entry.resize(access.indices.len(), BTreeSet::new());
    }
    let opaque_for_data = opaque_dims.entry(access.data).or_default();
    for (dim, ix_expr) in access.indices.iter().enumerate() {
        // TASK-0373: a data-dependent (gather) index makes this dim
        // OPAQUE — do not attribute any iv to it (the inner ivs of
        // `col_idx[i][k]` belong to col_idx, NOT to the outer array x).
        // An empty iv set drives the whole-array broadcast in
        // `compute_partition_bounds_with_dim_prefix`. Opacity is sticky:
        // a previously-opaque dim is cleared and skipped even if THIS
        // access indexes it affinely (whole-array must still serve the
        // gather sibling).
        if opaque_for_data.contains(&dim) {
            continue;
        }
        if expr_contains_dataref_or_call(ix_expr) {
            opaque_for_data.insert(dim);
            entry[dim].clear();
            continue;
        }
        collect_ivs_from_expr(ix_expr, name_iter_vars, &mut entry[dim]);
    }
}

/// TASK-0302 helper: apply the data-dim-prefix filter to derive per-Xfer
/// `IterTile::bounds` from the partition state.
///
/// Returns:
/// - `None` — no observed dim info for this data (no access on the
///   symbol, OR every access is a bare-aggregate `indices.is_empty()`
///   reference, OR a synthetic `DataflowEdge::new` fixture with empty
///   indices). The caller must then fall back to the *pre-TASK-0301*
///   "every partitioned iv applies" behaviour. This contract preserves
///   every shipped pre-TASK-0301 test fixture verbatim.
/// - `Some(bounds)` — applied the per-dim contiguous-prefix logic. The
///   returned Vec is in *data-dim* order. An empty Vec means the data's
///   partition-covered dims do not form a contiguous prefix starting at
///   dim 0 (the 07-matmul `b[k][j]` × `partition=blocks2d(i,j)` case:
///   only dim 1 is partition-covered, dim 0 is not, so no safe slice is
///   possible without an `iv → dim` mapping on `wait_slice`) — the
///   caller drops the tile to whole-array broadcast, the same shape
///   TASK-0301's i-membership filter already used for the
///   `partition=workers(i)` × `b[k][j]` 1D case.
///
/// ### Why dim-order, not partition_axis_order
///
/// `wait_slice`'s axis-mapping convention is `tile.bounds[i].iter_var
/// ↔ data.dim[i]` (row-major / nest-order). Emitting in dim order is
/// what makes that convention hold. The two orders coincide on every
/// shipped M5 cell because every cell's partition axes ARE the data's
/// dim-0 (`workers` on `[y][x]`, `[i][k]`, `[i][j]`) OR dim-0+dim-1
/// nest-order (`blocks2d` on `[y][x]`), so this rewrite is
/// observationally identical for them. The change bites when partition
/// axes are *sparse over the data's dim space* — the 07-matmul
/// `b[k][j]` × `partition=blocks2d(i,j)` shape that motivates this task.
///
/// ### Ambiguity handling
///
/// If a single dim is observed indexed by *multiple* partitioned ivs
/// (e.g. `a[i+j]` where both `i` and `j` are partitioned), the slicing
/// shape is ambiguous (which partition is "the" partition of this dim?).
/// The conservative choice is `Some(empty)` → whole-array broadcast.
/// Today's grammar + every shipped schedule keeps the iv-per-dim
/// cardinality at 1; this branch is defensive.
///
/// `partition_axis_order` is consulted only when `dim_iv_map.get(&data)`
/// is `None` (fall-back path in the caller), so it is intentionally NOT
/// a parameter to this function.
///
/// ### Advisory `NUC_TRACE` diagnostic (TASK-0424)
///
/// Returning EMPTY bounds means "no precise per-worker slice; the caller
/// emits a whole-array broadcast". This is the value-correct conservative
/// superset — NOT a soundness bug and NOT rejected (cf. PRD §8.6, which
/// this task reconciled to match the code). It happens for THREE distinct
/// reasons, and the advisory trace below names which one fired:
///
/// 1. **No partition-covered dim** — terminal `Some(bounds)` with
///    `bounds.is_empty()`: no dim carries a partitioned iv (e.g. a
///    non-affine / data-dependent / opaque-dim index recorded as an
///    empty iv set by `record_access_per_dim`, or a dim only indexed by
///    non-partitioned ivs).
/// 2. **Ambiguous multi-iv** — a single dim is indexed by more than one
///    partitioned iv.
/// 3. **Sparse-after-hole** — a partition-covered dim appears AFTER an
///    uncovered dim, so the coverage is not a contiguous dim-0 prefix.
///
/// The trace is purely advisory: this function still returns `Some(...)`
/// on every path exactly as before, and with `NUC_TRACE` unset it is
/// byte-silent (the e2e snapshot is unaffected). Hard-failing here would
/// be the panic-on-valid-input antipattern — correctness holds.
pub(super) fn compute_partition_bounds_with_dim_prefix(
    data: DataId,
    dim_iv_map: &BTreeMap<DataId, Vec<BTreeSet<IterVar>>>,
    partition_ranges: &BTreeMap<IterVar, BTreeMap<WorkerId, std::ops::Range<i64>>>,
    worker: WorkerId,
) -> Option<Vec<(IterVar, std::ops::Range<i64>)>> {
    let per_dim = dim_iv_map.get(&data)?;
    if per_dim.is_empty() {
        return None;
    }
    // Resolve per-dim coverage: at each dim, find the (unique) partitioned
    // iv that indexes it on this worker.
    let mut per_dim_cover: Vec<Option<(IterVar, std::ops::Range<i64>)>> =
        Vec::with_capacity(per_dim.len());
    for iv_set in per_dim {
        let partitioned: Vec<IterVar> = iv_set
            .iter()
            .copied()
            .filter(|iv| partition_ranges.contains_key(iv))
            .collect();
        match partitioned.len() {
            0 => per_dim_cover.push(None),
            1 => {
                let iv = partitioned[0];
                let range = partition_ranges
                    .get(&iv)
                    .and_then(|m| m.get(&worker))
                    .cloned();
                per_dim_cover.push(range.map(|r| (iv, r)));
            }
            _ => {
                // Ambiguous (multiple partitioned ivs at the same dim).
                // Defensive whole-array broadcast.
                crate::nuc_trace!(
                    "transfer_inject::compute_partition_bounds_with_dim_prefix: degrade to \
                     whole-array broadcast (data={data:?}, worker={worker:?}); reason: \
                     ambiguous multi-iv (a single dim is indexed by {n} partitioned ivs) — \
                     value-correct conservative fallback, no precise per-worker slice emitted",
                    n = partitioned.len(),
                );
                return Some(Vec::new());
            }
        }
    }
    // Walk dims in order, accept contiguous prefix from dim 0; reject
    // sparse coverage (a covered dim AFTER a hole).
    let mut bounds: Vec<(IterVar, std::ops::Range<i64>)> = Vec::new();
    let mut hit_hole = false;
    for slot in per_dim_cover {
        match (slot, hit_hole) {
            (Some(b), false) => bounds.push(b),
            (None, _) => hit_hole = true,
            (Some(_), true) => {
                // Sparse coverage (e.g. b[k][j] with partition on j but
                // not on k): wait_slice's dim-i ↔ bounds[i] convention
                // would silently mis-map. Drop to whole-array.
                crate::nuc_trace!(
                    "transfer_inject::compute_partition_bounds_with_dim_prefix: degrade to \
                     whole-array broadcast (data={data:?}, worker={worker:?}); reason: \
                     sparse-non-prefix coverage (a partition-covered dim follows an \
                     uncovered dim, so coverage is not a contiguous dim-0 prefix) — \
                     value-correct conservative fallback, no precise per-worker slice emitted",
                );
                return Some(Vec::new());
            }
        }
    }
    if bounds.is_empty() {
        // No dim carries a partition-covered iv with a per-worker range on
        // this worker. Three sub-causes collapse here (all value-correct):
        //   (i)   the index is non-affine / data-dependent / opaque
        //         (recorded as an empty iv set by `record_access_per_dim`);
        //   (ii)  every covering iv is unpartitioned;
        //   (iii) the (unique) partitioned iv has no range entry for THIS
        //         worker (the `1 =>` arm above mapped it to `None`).
        // Falls through to whole-array broadcast.
        crate::nuc_trace!(
            "transfer_inject::compute_partition_bounds_with_dim_prefix: degrade to \
             whole-array broadcast (data={data:?}, worker={worker:?}); reason: \
             no partition-covered dim (non-affine/opaque index, all covering ivs \
             unpartitioned, or a partitioned iv lacks a range on this worker) — \
             value-correct conservative fallback, no precise per-worker slice emitted",
        );
    }
    Some(bounds)
}

/// TASK-0306: order an `inject_halo_strip_xfers` strip tile by data-dim
/// position (matching `wait_slice`'s `tile.bounds[i] ↔ data.dim[i]`
/// convention).
///
/// `inject_halo_strip_xfers` constructs each strip tile from the
/// partition-pair `(outer_iv, inner_iv)` and the per-axis halo band
/// arithmetic. The pre-cycle-133 emit hard-coded `[(outer_iv, ...),
/// (inner_iv, ...)]` — correct ONLY when the halo-bearing data is
/// indexed `[outer_iv][inner_iv]` (outer-axis-leading) AND both ivs
/// form a contiguous prefix of the data's dims. Every shipped schedule
/// (05/distributed-2d's `img_in[y][x]` × `partition=blocks2d(y, x)`)
/// is in that safe regime; the two open shapes this helper guards
/// against are:
///
/// 1. **Inner-axis-leading partition** — data indexed `[inner_iv]
///    [outer_iv]`. `outer_dim = 1`, `inner_dim = 0`; the emit must
///    flip to `[(inner_iv, ...), (outer_iv, ...)]`.
/// 2. **Non-prefix data layout** — data indexed `[k][inner_iv]`
///    where `k` is not partitioned. `outer_dim = None`; the emit
///    must drop to a whole-array push (empty bounds) rather than
///    silently mis-map dim 0 to the outer iv.
///
/// ### Fall-back: no observed dim info
///
/// Synthetic test fixtures built via `DataflowEdge::new` carry empty
/// `data_in_access` indices, so `walk_data_dim_iv_map` records the
/// data with `Some(vec![])` (an empty per-dim Vec). Production
/// callers always observe accesses with non-empty indices (TASK-0150
/// populates them at `build_acfg` time). We treat both `None` (data
/// not in the map at all) and `Some(empty)` uniformly as "no
/// observed dim info" and return the pre-cycle-133 default ordering
/// so the existing `halo_strip_synth.rs` fixtures (positive_3x3 /
/// positive_2x2 / determinism / placement) stay byte-identical. The
/// `None` branch is defensive — no production caller reaches it.
///
/// ### Ambiguity (both ivs at same dim)
///
/// `a[outer_iv + inner_iv]` is theoretically possible (composite
/// index expression); not seen in canonical AlgoIR. Defensive drop to
/// whole-array (empty bounds), same policy as
/// [`compute_partition_bounds_with_dim_prefix`].
pub(super) fn order_halo_strip_bounds_by_data_dim(
    data: DataId,
    outer_iv: IterVar,
    outer_range: std::ops::Range<i64>,
    inner_iv: IterVar,
    inner_range: std::ops::Range<i64>,
    data_dim_iv_map: &BTreeMap<DataId, Vec<BTreeSet<IterVar>>>,
) -> Vec<(IterVar, std::ops::Range<i64>)> {
    // Fall-back hot path: synthetic fixtures + the defensive
    // missing-entry case. Build the default order LAZILY (no clones
    // on the canonical-indexed-access branches below).
    //
    // TASK-0315: emit a `NUC_TRACE`-gated diagnostic on the fall-back
    // so a future engineer who unwittingly constructs a synthetic
    // fixture via `DataflowEdge::new` (empty `data_in_access`) gets
    // an observable signal that the cycle-133 axis-mapping defence
    // is BYPASSED on this call. Production callers always observe
    // accesses (`walk_data_dim_iv_map` records non-empty per-dim
    // sets), so this trace fires only for synthetic test paths.
    let per_dim = match data_dim_iv_map.get(&data) {
        Some(p) if !p.is_empty() => p,
        _ => {
            crate::nuc_trace!(
                "transfer_inject::order_halo_strip_bounds_by_data_dim: fall-back to default \
                 order (data={data:?}, outer_iv={outer_iv:?}, inner_iv={inner_iv:?}); \
                 data_dim_iv_map entry is {entry} — axis-mapping defence BYPASSED for this \
                 call (expected only on synthetic fixtures built via DataflowEdge::new)",
                entry = match data_dim_iv_map.get(&data) {
                    None => "absent",
                    Some(_) => "Some(empty)",
                }
            );
            return vec![(outer_iv, outer_range), (inner_iv, inner_range)];
        }
    };
    let outer_dim = per_dim.iter().position(|s| s.contains(&outer_iv));
    let inner_dim = per_dim.iter().position(|s| s.contains(&inner_iv));
    match (outer_dim, inner_dim) {
        (Some(od), Some(id)) if od == id => {
            // TASK-0424 (architect P2): advisory trace on the structural
            // whole-array degradation, parallel to the sibling
            // `compute_partition_bounds_with_dim_prefix`. Ambiguity: both
            // halo ivs index the SAME data dim (e.g. `a[outer + inner]`),
            // so no unambiguous per-dim slice exists.
            crate::nuc_trace!(
                "transfer_inject::order_halo_strip_bounds_by_data_dim: degrade to \
                 whole-array broadcast (data={data:?}, outer_iv={outer_iv:?}, \
                 inner_iv={inner_iv:?}); reason: ambiguous (both ivs index the same \
                 data dim {od}) — value-correct conservative fallback",
            );
            Vec::new()
        }
        (Some(od), Some(id)) if od < id => {
            vec![(outer_iv, outer_range), (inner_iv, inner_range)]
        }
        (Some(_), Some(_)) => {
            vec![(inner_iv, inner_range), (outer_iv, outer_range)]
        }
        _ => {
            // TASK-0424 (architect P2): advisory trace on the structural
            // whole-array degradation, parallel to the sibling. Non-prefix:
            // at least one of the two halo ivs does not index any observed
            // data dim (e.g. `[k][inner_iv]` with `k` unpartitioned, so
            // `outer_dim` is None), so a safe outer/inner slice ordering
            // cannot be derived.
            crate::nuc_trace!(
                "transfer_inject::order_halo_strip_bounds_by_data_dim: degrade to \
                 whole-array broadcast (data={data:?}, outer_iv={outer_iv:?}, \
                 inner_iv={inner_iv:?}); reason: non-prefix (outer_dim={outer_dim:?}, \
                 inner_dim={inner_dim:?}; at least one halo iv indexes no observed data \
                 dim) — value-correct conservative fallback",
            );
            Vec::new()
        }
    }
}

/// Recursively collect every `IrExpr::Ident(name)` whose `name` resolves
/// to an `IterVar` via `name_iter_vars`, accumulating into `out`. Const
/// idents (lookup misses) are silently ignored — they are partition-
/// invariant and don't contribute to axis-mapping.
///
/// TASK-0373: the caller [`record_access_per_dim`] short-circuits a
/// data-dependent dim (one whose index contains a `DataRef`/`Call`)
/// to OPAQUE *before* calling this function, so the `DataRef`/`Call`
/// arms below are now UNREACHABLE on the production path. They are
/// retained as a fail-safe: were a future caller to invoke this on a
/// gather subexpression directly, descending would mis-attribute the
/// inner ivs — so the arms record NOTHING (conservative whole-array),
/// matching the opacity contract rather than the pre-TASK-0373
/// "defensive descent" that caused the mis-attribution bug.
pub(super) fn collect_ivs_from_expr(
    expr: &IrExpr,
    name_iter_vars: &BTreeMap<String, IterVar>,
    out: &mut BTreeSet<IterVar>,
) {
    match expr {
        IrExpr::IntLit(_) => {}
        IrExpr::Ident(name) => {
            if let Some(iv) = name_iter_vars.get(name) {
                out.insert(*iv);
            }
        }
        IrExpr::Neg(inner) => collect_ivs_from_expr(inner, name_iter_vars, out),
        IrExpr::BinOp(_, lhs, rhs) => {
            collect_ivs_from_expr(lhs, name_iter_vars, out);
            collect_ivs_from_expr(rhs, name_iter_vars, out);
        }
        IrExpr::DataRef(_) => {
            // TASK-0373: UNREACHABLE on the production path —
            // `record_access_per_dim` marks any dim containing a
            // `DataRef` OPAQUE and never calls this function on it.
            // Record NOTHING: a data-dependent (gather) index must NOT
            // attribute its inner ivs to the outer array (doing so was
            // the pre-TASK-0373 mis-attribution bug). An empty
            // contribution drives the conservative whole-array
            // broadcast — the only sound transfer for a gather read.
        }
        IrExpr::Call { .. } => {
            // TASK-0373: UNREACHABLE on the production path (same
            // reasoning as the `DataRef` arm — a Call inside an index
            // is also data-dependent and short-circuited to OPAQUE by
            // `record_access_per_dim`). Record nothing.
        }
    }
}
