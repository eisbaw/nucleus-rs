use super::*;

// --------------------------------------------------------------------
// TASK-0324 cycle-144 AC#2 — silent-elision-risk validator
// --------------------------------------------------------------------

/// Walk every Operation in `root` and reject the same-worker-pair
/// elision + consumer-reads-outside-local-slice shape with a typed
/// [`TransferInjectError::SameSetSilentElisionRisk`]. Called from
/// [`inject_transfers`] BEFORE the recursive emission walk so the
/// existing same-worker short-circuits fire only after this validator
/// has certified the elision as safe.
///
/// ## Two elision sites covered by one validator
///
/// The validator defends against two structurally identical same-
/// worker elision sites:
///
/// 1. **Whole-set elision** (grep-witness anchor:
///    `if producer_workers == &consumer_workers` inside
///    `build_waits_for_op` — `continue; no transfer`): fires when the
///    producer and consumer worker sets are equal.
///
/// 2. **Per-element fan-out elision** (grep-witness anchor: `if src
///    == dst` inside `build_waits_for_op`'s cartesian-product fan-
///    out — `continue;`): fires for every worker in
///    `producer_workers ∩ consumer_workers`, even when the sets are
///    not equal. Example: producer = {w0..w3}, consumer = {w0..w3,
///    w4} — the four self-pairs `(w_i, w_i)` are skipped per-element
///    while the cross pairs to/from w4 are emitted normally. Each
///    skipped self-pair has the same silent-miscompile risk profile
///    as a whole-set elision restricted to that worker.
///
/// The validator therefore fires when
/// `producer_workers ∩ consumer_workers` is non-empty — generalising
/// the original cycle-144 set-equality check to cover the per-element
/// case (TASK-0325 cycle-145 reviewer P1.1 fold-back from cycle 144).
///
/// ## Discriminator (axis-by-axis against producer's write pattern)
///
/// For every `(op, data_id)` pair where the consumer (`op`) and the
/// producer (`producers_by_data[data_id]`) share at least one common
/// worker, the elision the same-worker short-circuit performs is
/// correct iff for every axis `k` of the data:
///
/// - If the PRODUCER's `data_out_access.indices[k]` is a bare
///   [`IrExpr::Ident`] whose name resolves to an iv in
///   `partition_iter_vars` (i.e. axis `k` is partitioned at the
///   producer — worker `w` writes only its own partition's k-th slot),
///   then the CONSUMER's `data_in_access.indices[k]` MUST be a bare
///   [`IrExpr::Ident`] with the same name (the consumer must read its
///   own worker's k-th slot, not an arbitrary one). Same-name
///   comparison is sufficient because: (a) producer and consumer
///   share the same worker set; (b) when both ops live in the same
///   `Repeat`-named outer scope, the iv name resolves to the same
///   [`IterVar`] in both; (c) when the consumer's enclosing scope
///   uses a different iv name from the producer's (e.g. the
///   06/distributed2 `vm` vs producer's `hy`), the names differ AND
///   the read references a non-matching slice — both wrong reasons to
///   elide.
///
/// - If the producer's write index on axis `k` is NOT a bare Ident
///   matching a partition iv (e.g. a `IntLit`, an arithmetic
///   expression, or a bare Ident referencing a non-partition iv),
///   axis `k` is NOT partition-sliced at the producer and EVERY
///   worker owns the full k-th axis of the data — the consumer's
///   read on this axis can be anything.
///
/// ## Why per-axis against the producer's write pattern, not the
/// consumer's scope
///
/// An earlier formulation rejected any consumer-read iv that wasn't
/// the consumer's enclosing partition iv. That over-rejected the
/// accumulator-self-read shape `tmp[hy][hx] <-- hblur_acc(...,
/// tmp[hy][hx])` (06/distributed): the consumer reads `tmp[hy][hx]`
/// where `hy` IS the partition iv but `hx` is the inner sweep — yet
/// the producer ALSO writes `tmp[hy][hx]`, so worker `w_i` reads
/// EXACTLY what it wrote. The per-axis check sees `P_0 == C_0 ==
/// Ident("hy")` (axis 0 is partitioned and aligned) and `P_1 ==
/// Ident("hx") which is NOT a partition iv` (axis 1 is not
/// partitioned at all) — both safe.
///
/// For 06/distributed2 the same machinery fires correctly:
/// `P_0 == Ident("hy")` is a partition iv, but `C_0 == Ident("vm")` —
/// names differ, so the consumer reads a non-aligned slot on axis 0
/// → reject.
pub(super) fn check_no_silent_elision_risk(
    root: &ACFGNode,
    producers_by_data: &BTreeMap<DataId, BTreeSet<WorkerId>>,
    partition_iter_vars: &BTreeSet<IterVar>,
    name_iter_vars: &BTreeMap<String, IterVar>,
    name_data: &BTreeMap<String, DataId>,
    producer_writes: &BTreeMap<DataId, DataAccess>,
) -> Result<(), TransferInjectError> {
    // Producer-write index is built by the caller via
    // `collect_producer_writes` and shared with `InjectCtx` so the
    // AC#3 same-set classifier inside `build_waits_for_op` uses the
    // SAME index without re-walking the tree. Per PRD §6.2.1 single-
    // assignment (enforced upstream by `algo::lower` — grep
    // `LowerErrorKind::DoubleAssignment` (defined in `algo::ir`, raised
    // in `algo::lower`) as the witness for the single-assignment rule),
    // every data symbol has at most ONE
    // writer Operation, modulo accumulator self-writes (an op whose
    // `data_in` and `data_out` are the same DataId at the same
    // indices, e.g. `c[i][j] <-- madd(c[i][j], ...)` — exercised by
    // 07-matmul's reference shape). Accumulator self-writes carry the
    // SAME `data_out_access` on every iteration by construction, so
    // the lexically-first occurrence is authoritative: the
    // `or_insert_with` inside `collect_producer_writes` is sound only
    // under (§6.2.1 single-assignment) ∧ (accumulator-self-writes
    // carry the same access on every iteration). Reviewer P2.5
    // cycle-144 fold-back.
    check_no_silent_elision_risk_inner(
        root,
        producers_by_data,
        partition_iter_vars,
        name_iter_vars,
        name_data,
        producer_writes,
    )
}

pub(super) fn collect_producer_writes(node: &ACFGNode, out: &mut BTreeMap<DataId, DataAccess>) {
    match node {
        ACFGNode::Sequence(children) => {
            for c in children {
                collect_producer_writes(c, out);
            }
        }
        ACFGNode::Repeat { body, .. } => collect_producer_writes(body, out),
        ACFGNode::Operation(op) => {
            for edge in &op.dataflow.edges {
                if let Some(out_access) = &edge.data_out_access {
                    out.entry(out_access.data)
                        .or_insert_with(|| out_access.clone());
                }
            }
        }
        ACFGNode::Xfer(_) | ACFGNode::Sync(_) => {}
    }
}

pub(super) fn check_no_silent_elision_risk_inner(
    node: &ACFGNode,
    producers_by_data: &BTreeMap<DataId, BTreeSet<WorkerId>>,
    partition_iter_vars: &BTreeSet<IterVar>,
    name_iter_vars: &BTreeMap<String, IterVar>,
    name_data: &BTreeMap<String, DataId>,
    producer_writes: &BTreeMap<DataId, DataAccess>,
) -> Result<(), TransferInjectError> {
    // TASK-0328 cycle-154 architect P2.1 fold-back: `enclosing_ivs`
    // threading dropped end-to-end. After clause (1) removal at the
    // leaf, no consumer in this function family consults the
    // enclosing-iv stack — the per-axis discriminator works on
    // producer's write pattern vs consumer's read pattern, not on
    // either side's enclosing scope.
    match node {
        ACFGNode::Sequence(children) => {
            for c in children {
                check_no_silent_elision_risk_inner(
                    c,
                    producers_by_data,
                    partition_iter_vars,
                    name_iter_vars,
                    name_data,
                    producer_writes,
                )?;
            }
            Ok(())
        }
        ACFGNode::Repeat { body, .. } => check_no_silent_elision_risk_inner(
            body,
            producers_by_data,
            partition_iter_vars,
            name_iter_vars,
            name_data,
            producer_writes,
        ),
        ACFGNode::Operation(op) => check_op_no_silent_elision_risk(
            op,
            producers_by_data,
            partition_iter_vars,
            name_iter_vars,
            name_data,
            producer_writes,
        ),
        ACFGNode::Xfer(_) | ACFGNode::Sync(_) => Ok(()),
    }
}

pub(super) fn check_op_no_silent_elision_risk(
    op: &Operation,
    producers_by_data: &BTreeMap<DataId, BTreeSet<WorkerId>>,
    partition_iter_vars: &BTreeSet<IterVar>,
    name_iter_vars: &BTreeMap<String, IterVar>,
    name_data: &BTreeMap<String, DataId>,
    producer_writes: &BTreeMap<DataId, DataAccess>,
) -> Result<(), TransferInjectError> {
    let consumer_workers: BTreeSet<WorkerId> = op.workers.clone();

    // TASK-0328 cycle-154: clause (1) (the "no partition iv on consumer
    // scope → safe elision" short-circuit) REMOVED as empirically
    // over-lenient. The original reasoning was "every worker owns the
    // full data", which conflates the consumer's enclosing scope with
    // what each worker has in memory. If the PRODUCER wrote with a
    // partition iv (each worker has only its band), then the CONSUMER
    // at top-level reading `tmp[c0][c1]` (constant indices) reads a
    // slice it does not own — silent miscompile. The per-axis check
    // below correctly catches this case: producer's axis-0 = Ident(hy)
    // = partition iv → p_iv = Some(hy); consumer's axis-0 = IntLit(c0)
    // → c_iv = None ≠ Some(hy) → returns Some(reason). Test
    // `task0328_ac2_positive_partition_producer_topfile_consumer`
    // pins the rejection. The `enclosing_ivs` parameter was dropped
    // entirely (cycle-154 architect P2.1 fold-back): no longer used
    // at this site and the inner walker still tracks it for
    // Repeat-enclosing-scope recursion correctness.

    let mut seen: BTreeSet<DataId> = BTreeSet::new();
    for edge in &op.dataflow.edges {
        for (i, &data_id) in edge.data_in.iter().enumerate() {
            if !seen.insert(data_id) {
                continue;
            }
            let producer_workers = match producers_by_data.get(&data_id) {
                Some(p) => p,
                None => continue, // No recorded producer.
            };
            // TASK-0325 cycle-145: both elision sites are checked
            // together. The whole-set short-circuit (grep-witness:
            // `if producer_workers == &consumer_workers` inside
            // `build_waits_for_op`) fires for the whole-set case.
            // The per-element skip (grep-witness: `if src == dst`
            // inside `build_waits_for_op`'s cartesian-product fan-out)
            // fires for EVERY worker in the intersection of
            // producer_workers and consumer_workers — that is, even
            // when the sets are not equal (e.g. producer={w0..w3},
            // consumer={w0..w3, w4}: the four self-pairs (w_i, w_i)
            // are skipped per-element while the cross pairs to/from
            // w4 are emitted normally). Both elisions risk a silent
            // miscompile under the same per-axis safety conditions,
            // so the validator fires when the intersection is non-
            // empty.
            let same_worker_set: BTreeSet<WorkerId> = producer_workers
                .intersection(&consumer_workers)
                .copied()
                .collect();
            if same_worker_set.is_empty() {
                continue; // No same-worker elision happens anywhere.
            }

            // Need the producer's write access to drive the per-axis
            // discriminator. If we don't have it (synthetic test
            // fixtures that omit data_out_access, or a producer not
            // captured by `collect_producer_writes`), conservatively
            // continue: the validator is additive over the existing
            // behaviour, and missing producer access means we cannot
            // distinguish safe from unsafe — falling back to the
            // pre-TASK-0324 elision is acceptable for synthetic-only
            // fixtures because those don't exercise the silent-
            // miscompile code path in production.
            let prod_access = match producer_writes.get(&data_id) {
                Some(p) => p,
                None => continue,
            };

            // Classify whether the existing same-worker elision is
            // safe for this (op, data) pair. The classifier is shared
            // with `build_waits_for_op`'s same-set short-circuit so
            // the validator and the emission decision use the SAME
            // predicate by construction (no drift between AC#2
            // rejection and AC#3 emit). `Some(reason)` means the
            // elision is UNSAFE — the consumer reads outside the
            // local producer's partition slice.
            let unsafe_reason = same_set_elision_unsafe_reason(
                edge,
                data_id,
                prod_access,
                partition_iter_vars,
                name_iter_vars,
            );

            if let Some(reason) = unsafe_reason {
                // TASK-0324 cycle-147 AC#3 lift: when the producer
                // and consumer worker sets are IDENTICAL, the AC#3
                // codegen extension at `build_waits_for_op`'s same-
                // set short-circuit now FALLS THROUGH to the
                // cartesian-product fan-out and emits cross-worker
                // pairs. So the previously-silent elision is now a
                // real cross-worker transfer; no need to reject.
                //
                // The partial-overlap case (TASK-0325 — different
                // sets sharing some workers, the `if src == dst`
                // per-element skip inside the cartesian fan-out)
                // is NOT in cycle 147's AC#3 scope; the existing
                // `if src == dst { continue; }` still elides those
                // per-element self-pairs unconditionally. So the
                // validator continues to reject partial-overlap
                // unsafe shapes — a future cycle that extends AC#3
                // to the per-element-fan-out case will lift this
                // rejection too. No in-tree schedule exercises the
                // partial-overlap path today.
                //
                // TASK-0333 cycle-155 audit confirmed (paired-lift
                // sweep of the cycle-154 clause-(1) removal): the
                // partial-overlap arm has NO clause-(1) analog to
                // remove. The emit-site per-element skip
                // (`if src == dst { continue; }` in `build_waits_for_op`'s
                // cartesian fan-out) is unconditional — no consumer-
                // scope gate exists there to elide. This validator
                // rejection IS the load-bearing safety net for the
                // partial-overlap arm. Regression pin:
                // `task0333_ac1_partial_overlap_partition_producer_topfile_consumer_rejects`.
                if producer_workers == &consumer_workers {
                    continue; // AC#3 handles this in cycle 147.
                }

                // FUTURE-WORK (TASK-0333 cycle-155b architect P3.1):
                // extend cycle-147 AC#3 to per-element fan-out for
                // partial-overlap unsafe shapes (lift this rejection
                // when an in-tree schedule needs it). Grep-anchor for
                // scanning the latent extension surface.
                let data_name = name_data
                    .iter()
                    .find_map(|(n, id)| (*id == data_id).then_some(n.as_str()))
                    .unwrap_or("<unknown>");
                // The user-facing `message` is tracker-ID-free
                // (TASK-0455.06). The internal forward-links are TASK-0324
                // (set-equality elision, lifted by its AC#3) and TASK-0325
                // (per-element fan-out elision, still defended) — kept here
                // in the comment and on the `SameSetSilentElisionRisk`
                // variant doc, NOT in the surfaced diagnostic.
                let message = format!(
                    "data `{data_name}` (id {data_id:?}, edge.data_in index {i}): \
                     producer worker set ({producer_workers:?}) and consumer \
                     worker set ({consumer_workers:?}) overlap on the \
                     same-worker pairs {same_worker_set:?}, which the \
                     cartesian-product fan-out skips per-element, AND \
                     {reason}. Without a cross-worker transfer this elides \
                     into a silent miscompile (the consumer reads outside \
                     the local producer's partition slice). Fix: remove the \
                     producer/consumer worker-set overlap, or switch the \
                     consumer to a different partition loop variable so it \
                     reads only its own slice."
                );
                return Err(TransferInjectError::SameSetSilentElisionRisk {
                    data: data_id,
                    message,
                });
            }
        }
    }
    Ok(())
}

/// Shared classifier for the AC#2 validator + AC#3 emission decision.
/// Returns `Some(reason)` when the existing same-worker elision (for
/// a single `(edge, data_id)` consumer side, given the producer's
/// `prod_access`) would be a SILENT MISCOMPILE — i.e. the consumer
/// reads outside the local producer's partition slice. Returns `None`
/// when the elision is correct (consumer reads only its own slice).
///
/// Caller is responsible for the upstream gates:
/// - non-empty worker-set intersection,
/// - non-empty enclosing partition-iv stack,
/// - producer's `data_out_access` present (`prod_access`),
/// - non-empty `prod_access.indices`.
///
/// The per-axis discriminator works axis-by-axis against the
/// producer's write pattern; see the module docs and the
/// `check_no_silent_elision_risk` doc comment for the full rationale.
pub(super) fn same_set_elision_unsafe_reason(
    edge: &crate::acfg::DataflowEdge,
    data_id: DataId,
    prod_access: &DataAccess,
    partition_iter_vars: &BTreeSet<IterVar>,
    name_iter_vars: &BTreeMap<String, IterVar>,
) -> Option<String> {
    for access in edge.data_in_access.iter() {
        if access.data != data_id {
            continue;
        }

        // If the producer carries indices but the consumer carries
        // none (a whole-array read of partitioned data), reject —
        // the worker does NOT own the whole array.
        if access.indices.is_empty() && !prod_access.indices.is_empty() {
            // Determine if ANY producer axis references a partition
            // iv (anywhere in the IrExpr tree — bare Ident, arithmetic
            // like `hy*2`, `hy+1`, `-hy`, etc.). If every axis is
            // non-partition (so producer wrote whole-array), the
            // elision is still safe.
            //
            // TASK-0326 cycle-156: tightened from the bare-Ident-only
            // `ident_iv_in_set` predicate (which silently treated
            // `tmp[hy*2][hx]` as non-partitioned) to the recursive
            // tree-walking `expr_references_partition_iv`.
            let any_partitioned_axis = prod_access
                .indices
                .iter()
                .any(|p| expr_references_partition_iv(p, partition_iter_vars, name_iter_vars));
            if any_partitioned_axis {
                return Some(format!(
                    "consumer reads data as a whole array (no indices) while \
                     the producer writes with a partition iv on at least one \
                     axis (producer indices: {:?}); each worker owns only \
                     its partition slice",
                    prod_access.indices
                ));
            } else {
                continue;
            }
        }

        // Per-axis check.
        //
        // TASK-0326 cycle-156: the per-axis discriminator was
        // previously bare-Ident-only. The classifier returned
        // `Some(p_iv)` only when the producer's axis-k expression
        // was `IrExpr::Ident(name)` with `name` resolving to a
        // partition iv — arithmetic shapes like `tmp[hy*2][hx]`
        // or `tmp[hy+1][hx]` fell into the CONSERVATIVELY-NOT-
        // REJECTED path, treating the axis as whole-array and
        // skipping the constraint on the consumer. That was the
        // dormant under-conservative path filed as TASK-0326.
        //
        // The tightened rule:
        //   1. If the producer's axis-k expression CONTAINS any
        //      partition iv (anywhere in the IrExpr tree, walked
        //      recursively by `expr_references_partition_iv`),
        //      treat the axis as partition-sliced.
        //   2. The consumer's axis-k expression must then be
        //      STRUCTURALLY EQUAL to the producer's (IrExpr derives
        //      PartialEq + Eq). Same `Ident(hy)` is fine; the
        //      cycle-144 bare-Ident case is SUBSUMED. Same
        //      `BinOp(Mul, Ident(hy), IntLit(2))` is also fine.
        //      Anything else → reject.
        //   3. If the producer's axis-k expression does NOT
        //      reference a partition iv (`IntLit(c)`, non-iv
        //      `Ident(const_name)`, arithmetic over non-iv names):
        //      axis is whole-array at every worker; consumer is
        //      unconstrained.
        //
        // Safety direction (PRD bias toward fail-loud): structural
        // equality is the minimum sound discriminator. Over-rejection
        // is fail-loud (user sees a clear error and refactors);
        // under-rejection is silent miscompile. Cases where a
        // non-structural-equality consumer read is provably safe
        // (e.g. the access stays within the halo-extended tile)
        // are NOT accepted by this classifier — the halo-aware
        // escape valve is option B (documented; deferred to a
        // follow-up if an in-tree schedule trips it). The cases
        // that previously hit CONSERVATIVELY-NOT-REJECTED are:
        //   - `Call(...)` / `DataRef(...)` as producer indices: not
        //     used today; rejected upstream by `algo::lower`. Still
        //     handled by `expr_references_partition_iv`'s
        //     defensive-walk over `Call.args` and `DataRef.indices`
        //     in case the upstream gate changes.
        let n_axes = prod_access.indices.len().min(access.indices.len());
        for k in 0..n_axes {
            let p_partitioned = expr_references_partition_iv(
                &prod_access.indices[k],
                partition_iter_vars,
                name_iter_vars,
            );
            if !p_partitioned {
                // Producer's axis-k does not involve any partition
                // iv → whole-array on every worker → consumer's
                // axis-k read is unconstrained.
                continue;
            }
            // Producer's axis k IS partition-sliced. The consumer's
            // axis-k read must be STRUCTURALLY EQUAL to the
            // producer's. This is the minimum sound discriminator;
            // see the comment block above for the safety rationale.
            if prod_access.indices[k] != access.indices[k] {
                // The reason string is tracker-ID-free (TASK-0455.06);
                // the internal forward-link for the tightened structural
                // rule is TASK-0326 (cycle-156) — kept here in the
                // comment, not surfaced.
                return Some(format!(
                    "axis {k} is partition-sliced at the producer (writes \
                     at {:?}); consumer reads at {:?} which does not match \
                     structurally — worker reads a slice it does not own. \
                     The consumer's axis-k read expression must be \
                     structurally equal to the producer's write expression \
                     (a halo-aware escape valve accepting reads provably \
                     within the halo-extended tile is not yet implemented).",
                    prod_access.indices[k], access.indices[k]
                ));
            }
        }
    }
    None
}

/// Recursively walks `expr` and returns `true` iff any subexpression
/// is an `IrExpr::Ident(name)` whose resolved `IterVar` is in `set`.
///
/// Used by the per-axis discriminator in
/// `same_set_elision_unsafe_reason` to recognise partition-iv-bearing
/// producer indices — including arithmetic shapes like
/// `tmp[hy*2][hx]`, `tmp[hy+1][hx]`, or `tmp[-hy][hx]` that the
/// pre-cycle-156 bare-Ident `ident_iv_in_set` silently treated as
/// non-partitioned (the under-conservative dormant path filed as
/// TASK-0326).
///
/// Cases walked:
/// - `Ident(name)`: the partition-iv leaf detector.
/// - `IntLit(_)`: false — no iv reference possible.
/// - `Neg(e)`: recurse on `e`.
/// - `BinOp(_, l, r)`: recurse on `l` OR `r`.
/// - `DataRef(IndexedRef { indices, .. })`: defensive recurse over
///   every index. `algo::lower` rejects `DataRef` as a producer
///   index today, but the predicate stays sound if that upstream
///   gate ever changes (avoids a silent miscompile by construction).
/// - `Call { args, .. }`: defensive recurse over every arg. Same
///   upstream-rejected-today caveat as `DataRef`.
pub(super) fn expr_references_partition_iv(
    expr: &IrExpr,
    set: &BTreeSet<IterVar>,
    name_iter_vars: &BTreeMap<String, IterVar>,
) -> bool {
    match expr {
        IrExpr::Ident(name) => match name_iter_vars.get(name) {
            Some(iv) => set.contains(iv),
            None => false,
        },
        IrExpr::IntLit(_) => false,
        IrExpr::Neg(e) => expr_references_partition_iv(e, set, name_iter_vars),
        // A comparison is bool-valued (cannot appear in an index position
        // today); recurse into both operands defensively so the predicate
        // stays sound if a future bool-RHS path routes here
        // (TASK-0341.02.01.02 / S2).
        IrExpr::BinOp(_, l, r) | IrExpr::Compare(_, l, r) => {
            expr_references_partition_iv(l, set, name_iter_vars)
                || expr_references_partition_iv(r, set, name_iter_vars)
        }
        IrExpr::DataRef(idx_ref) => idx_ref
            .indices
            .iter()
            .any(|i| expr_references_partition_iv(i, set, name_iter_vars)),
        IrExpr::Call { args, .. } => args
            .iter()
            .any(|a| expr_references_partition_iv(a, set, name_iter_vars)),
    }
}
