//! (B') partition-policy fatality predicate + supporting band/partition
//! probes for halo inference.
//!
//! Carved from `halo_inference.rs` (TASK-0460, content-preserving
//! mega-file split) along the module-docstring "Strict vs advisory vs
//! partition-policy-aware entry points" seam. The single public-to-parent
//! entry is [`error_is_fatal_under_partition`], consumed by
//! [`super::apply_halo_inference_partition_aware`]. All other items are
//! private band/partition probes.

use crate::algo::{IndexedRef, IrExpr, IrStmt};
use crate::link::LinkedIR;
use crate::sched::ResolvedLoopOption;
use crate::passes::common::expr_contains_dataref_or_call;

use super::HaloInferenceError;
use super::walker::collect_iter_var_refs;

/// (B') per-variant fatality predicate. Split from the entry-point
/// loop so the per-variant rule is self-documenting + unit-testable
/// without recreating an `infer_halo_widths` walk.
///
/// See [`apply_halo_inference_partition_aware`] for the doc on the
/// per-variant split. Three rule shapes coexist:
/// - The precise variants (`NonAffineIndex` etc.) use `ivs_in_index`:
///   fatal iff a referenced iv is partitioned.
/// - `UnknownKernelInCall` falls back to the conservative
///   enclosing-`scope` rule (a fail-closed diagnostic for
///   inconsistently-constructed IR).
/// - TASK-0373 / TASK-0384: `DataDependentStride` (a data-dependent
///   READ) splits on its `is_scatter_rmw` flag:
///     - PURE GATHER (`false`): advisory — the transfer layer
///       broadcasts the whole gathered array, so the partition state is
///       irrelevant.
///     - SCATTER RMW (`true`): the cross-worker WRITE coordination is
///       served by the *replicate-full-array-per-worker +
///       element-wise-sum combine* shape (TASK-0343
///       `collect_accumulate_waits` + `render_wait_assign(accumulate)`)
///       — but ONLY when that shape is SOUND. It is sound iff the
///       scatter target (`ref_name`) replicates WHOLE-ARRAY to every
///       worker, which holds iff NO partitioned iv affinely indexes the
///       scatter target anywhere in the algorithm
///       ([`scatter_target_replicates_whole_array`]). That is the
///       INPUT-INDEX partition: each worker owns a disjoint slice of the
///       SOURCE (`input[i]`, `i` partitioned), scatters into its own
///       full-width private partial, and the partials sum. So:
///         - no enclosing-`scope` iv partitioned (single-worker): advisory
///           (the read needs no transfer at all);
///         - partitioned + scatter target replicates whole-array
///           (input-index partition): ADVISORY (TASK-0384 admits it);
///         - partitioned + scatter target is BAND-partitioned by a
///           partitioned iv (a bin-partition, e.g. `histogram[b]` with
///           `b` partitioned): FATAL — replicate-per-worker would lose
///           every scatter that lands outside the worker's bin band, so
///           the element-wise-sum combine is UNSOUND.
pub(super) fn error_is_fatal_under_partition(
    err: &HaloInferenceError,
    scope: &[String],
    ivs_in_index: &[String],
    linked: &LinkedIR,
) -> bool {
    match err {
        // Data-dependent READ address (a gather: `x[col_idx[i][k]]`).
        // The runtime cell the worker reads is determined by the loaded
        // index value, not the lexically-visible iv set — so a
        // partition-band slice of the gathered array cannot be computed
        // at compile time.
        //
        // TASK-0373: this is NO LONGER fatal under partition. The
        // transfer-injection pass marks the gathered array's
        // data-dependent dim OPAQUE
        // (`transfer_inject::record_access_per_dim`), which drives a
        // conservative WHOLE-ARRAY BROADCAST of that array to every
        // worker (`compute_partition_bounds_with_dim_prefix` returns
        // empty bounds for an opaque dim). With the whole array present
        // on each worker, ANY runtime index it loads is in range, so the
        // read is served correctly without a halo.
        //
        // SOUNDNESS COUPLING (load-bearing): this advisory relaxation is
        // valid ONLY because the transfer layer actually broadcasts the
        // whole array on the SAME data-dependence predicate
        // (`common::expr_contains_dataref_or_call`, shared by both
        // passes). The opacity is sticky in transfer_inject, so a dim
        // observed data-dependent on any access stays whole-array even
        // if a sibling affine access exists. The e2e byte-exact diff vs
        // `reference.bin` is the safety net: if x were NOT broadcast
        // whole, workers would read garbage and the diff would fail
        // loud.
        //
        // SCATTER GUARD (load-bearing — TASK-0373 + TASK-0384): halo
        // inference walks kernel-call ARGS (reads), so the index here is
        // always a READ. BUT a scatter read-modify-write
        // (`histogram[input[i]] <-- inc(histogram[input[i]])`) reads the
        // SAME symbol it writes by a data-dependent address — the RHS
        // read reaches this arm with `is_scatter_rmw == true`.
        //
        //   - pure gather (`is_scatter_rmw == false`): ADVISORY (relaxed;
        //     whole-array broadcast serves any READ index — TASK-0373).
        //   - scatter RMW (`is_scatter_rmw == true`) + no partitioned iv
        //     (single-worker): ADVISORY — the read needs no transfer.
        //   - scatter RMW + partitioned, scatter target replicates
        //     whole-array (INPUT-INDEX partition): ADVISORY. Each worker
        //     scatters its input slice into a private full-width partial;
        //     the TASK-0343 element-wise-sum combine reduces the partials
        //     correctly. This is the TASK-0384 admit shape.
        //   - scatter RMW + partitioned, scatter target BAND-partitioned
        //     by a partitioned iv (a BIN partition): FATAL. A worker
        //     owning only a bin band would silently drop every scatter
        //     that lands outside its band, so replicate-per-worker +
        //     element-wise-sum is UNSOUND. `scatter_target_replicates_
        //     whole_array` is the discriminator; `ref_name` names the
        //     scattered-into symbol carried on the error.
        HaloInferenceError::DataDependentStride {
            is_scatter_rmw,
            ref_name,
            ..
        } => {
            *is_scatter_rmw
                && scope.iter().any(|iv| iv_is_partitioned(linked, iv))
                && !scatter_target_replicates_whole_array(linked, ref_name)
        }
        // Link-invariant break: the kernel id was missing from
        // `name_kernels`. Never reachable from a link-valid IR.
        // Fall back to scope conservatively — the variant exists
        // primarily for fail-closed diagnostics on inconsistently-
        // constructed `(LinkedIR, ACFG)` pairs.
        HaloInferenceError::UnknownKernelInCall { .. } => {
            scope.iter().any(|iv| iv_is_partitioned(linked, iv))
        }
        // Precise variants: the iv set is populated from the
        // failing index expression at the error-push site. Fatal
        // iff at least one of those ivs is partitioned.
        HaloInferenceError::NonAffineIndex { .. }
        | HaloInferenceError::StridedAccessNotSupported { .. }
        | HaloInferenceError::MultipleIterVarsInIndex { .. }
        | HaloInferenceError::UnknownLoopVar { .. } => {
            ivs_in_index.iter().any(|iv| iv_is_partitioned(linked, iv))
        }
    }
}

/// Does the schedule's `loops` table tag this iv with a
/// `partition=` directive? Returns false for missing iv (no schedule
/// directive at all) and false for ivs whose directive carries only
/// non-partition options (`block=`, `pipeline=`, `reuse`).
///
/// Used by [`apply_halo_inference_partition_aware`] for the per-error
/// fatality decision.
fn iv_is_partitioned(linked: &LinkedIR, iv: &str) -> bool {
    linked
        .sched
        .loops
        .get(iv)
        .map(|d| {
            d.options
                .iter()
                .any(|o| matches!(o, ResolvedLoopOption::Partition(_)))
        })
        .unwrap_or(false)
}

/// TASK-0384 soundness discriminator for a SCATTER read-modify-write
/// under partition: does the scatter target symbol `target` replicate
/// WHOLE-ARRAY to every worker?
///
/// A scatter (`histogram[input[i]] <-- inc(histogram[input[i]])`) is
/// served under partition by the *replicate-full-array-per-worker +
/// element-wise-sum combine* shape (TASK-0343): each worker holds a
/// PRIVATE full-width copy of `target`, scatters its partition slice
/// into it, and the host sums the partials element-wise. That is sound
/// iff every worker really does hold the FULL array — i.e. `target` is
/// broadcast whole, never band-partitioned.
///
/// An OPAQUE (data-dependent) index dim is ONE of several conditions
/// under which the transfer layer broadcasts `target` whole-array
/// (`compute_partition_bounds_with_dim_prefix` also drops to whole-array
/// on all-`None` cover, sparse coverage, and multi-iv ambiguity);
/// opacity is sticky in `transfer_inject::record_access_per_dim`. Rather
/// than mirror that decision exactly, this discriminator is a
/// CONSERVATIVE approximation of the truly-unsound set: it returns
/// `false` (NOT whole-array ⇒ keep the scatter FATAL) iff ANY index
/// expression into `target` — on EITHER a write LHS or a read DataRef,
/// at any depth — affinely references a partitioned iv. That is a
/// SUPERSET of the genuinely-unsound shapes (it may keep FATAL a few
/// shapes the transfer layer would actually broadcast whole-array —
/// safe, never the reverse). Two unsound shapes it must reject:
///   - a partitioned iv indexes `target` affinely (`histogram[b]`, `b`
///     partitioned ⇒ band-partition ⇒ a worker owning only its band
///     drops out-of-band scatters); and
///   - a cross-band affine self-read alongside the scatter
///     (`h[input[i]] <-- inc(h[input[i]], h[i])`, `i` partitioned). Even
///     though `h[input[i]]` makes dim 0 opaque (so the transfer layer
///     WOULD broadcast `h` whole, NOT band it), the affine `h[i]` read
///     does not decompose under replicate-then-element-wise-sum — each
///     worker reads its OWN partial's `h[i]`, not the global — so the
///     combine is unsound. The affine `h[i]` index trips this predicate
///     and keeps it FATAL, which is correct. (This is the shape the unit
///     test pins; the rejection is NOT because `h` is banded — it is
///     not — but because an affine partitioned-iv index into the target
///     exists at all.)
///
/// Otherwise `target` is only ever data-dependently indexed (or
/// const/whole-aggregate), the transfer layer replicates it whole, and
/// the replicate-then-sum shape is sound: returns `true`.
///
/// Walks `linked.algo.stmts` (source-order vector; deterministic). Uses
/// `expr_contains_dataref_or_call` to skip data-dependent index dims
/// (opaque/whole-array — the SOUND dim) and `collect_iter_var_refs` +
/// `iv_is_partitioned` to detect an affine partitioned-iv index. Further
/// conservative: a non-affine but partition-iv-mentioning index (e.g.
/// `target[iv*iv]`) is treated as a banding access and keeps it FATAL.
pub(super) fn scatter_target_replicates_whole_array(linked: &LinkedIR, target: &str) -> bool {
    // The full lexical scope of iter-var names this algorithm could
    // bind, so `collect_iter_var_refs` recognises every loop var (the
    // recursion below does NOT carry a running scope — collecting the
    // union of all `For` vars up front is equivalent for the
    // "mentions a partitioned iv" question, since a partition directive
    // only exists for a declared loop var).
    let mut all_loop_vars: Vec<String> = Vec::new();
    collect_loop_vars(&linked.algo.stmts, &mut all_loop_vars);

    !algo_target_has_affine_partitioned_index(
        &linked.algo.stmts,
        target,
        &all_loop_vars,
        linked,
    )
}

/// Collect every `For` loop-variable name in the statement tree
/// (source order, including nested bodies). Used by
/// [`scatter_target_replicates_whole_array`] to populate the iter-var
/// scope for affine-index detection.
fn collect_loop_vars(stmts: &[IrStmt], out: &mut Vec<String>) {
    for s in stmts {
        if let IrStmt::For { var, body, .. } = s {
            out.push(var.clone());
            collect_loop_vars(body, out);
        }
    }
}

/// Returns `true` iff some indexed access to `target` (write LHS or read
/// DataRef, at any depth) carries an index expression that AFFINELY
/// mentions a partitioned iv — i.e. `target` would be band-partitioned
/// rather than replicated whole-array.
///
/// A data-dependent index dim (`expr_contains_dataref_or_call`) is
/// SKIPPED: that dim is opaque ⇒ whole-array broadcast (the sound
/// scatter case). Only a plain affine partitioned-iv index bands the
/// array.
fn algo_target_has_affine_partitioned_index(
    stmts: &[IrStmt],
    target: &str,
    loop_vars: &[String],
    linked: &LinkedIR,
) -> bool {
    for s in stmts {
        match s {
            IrStmt::Dataflow { lhs, rhs } => {
                if indexed_ref_bands_target(lhs, target, loop_vars, linked) {
                    return true;
                }
                // TASK-0384 review P3: the LHS index sub-expressions may
                // THEMSELVES nest a banding DataRef on `target`
                // (`other[ target[j] ] <-- ...`, `j` partitioned). The
                // RHS DataRef arm of `expr_bands_target` already descends
                // index sub-expressions; mirror it on the LHS so the
                // "at any depth, write LHS or read DataRef" contract in
                // the docstring actually holds and the LHS path is not an
                // asymmetric blind spot (additive-conservative — can only
                // keep MORE scatters FATAL, never admit a banding one).
                // This arm is unreachable in today's grammar so it has no
                // dedicated bite-test yet — TASK-0390.
                if lhs
                    .indices
                    .iter()
                    .any(|ix| expr_bands_target(ix, target, loop_vars, linked))
                {
                    return true;
                }
                if expr_bands_target(rhs, target, loop_vars, linked) {
                    return true;
                }
            }
            IrStmt::Effect { args, .. } => {
                for a in args {
                    if expr_bands_target(a, target, loop_vars, linked) {
                        return true;
                    }
                }
            }
            IrStmt::For { body, .. } => {
                if algo_target_has_affine_partitioned_index(body, target, loop_vars, linked) {
                    return true;
                }
            }
        }
    }
    false
}

/// Does this expression contain an indexed access to `target` whose
/// index affinely mentions a partitioned iv? Recurses into `Call` args,
/// `Neg`, `BinOp`, and `DataRef` index sub-expressions.
fn expr_bands_target(
    e: &IrExpr,
    target: &str,
    loop_vars: &[String],
    linked: &LinkedIR,
) -> bool {
    match e {
        IrExpr::IntLit(_) | IrExpr::Ident(_) => false,
        IrExpr::Neg(inner) => expr_bands_target(inner, target, loop_vars, linked),
        // A comparison can appear in a (bool-typed) RHS; a banding DataRef
        // may be buried in either integer operand (`flag <-- a[i+1] <=
        // b`), so descend into both (TASK-0341.02.01.02 / S2).
        IrExpr::BinOp(_, a, b) | IrExpr::Compare(_, a, b) => {
            expr_bands_target(a, target, loop_vars, linked)
                || expr_bands_target(b, target, loop_vars, linked)
        }
        IrExpr::DataRef(r) => {
            if indexed_ref_bands_target(r, target, loop_vars, linked) {
                return true;
            }
            // The index sub-expressions of a DataRef on a DIFFERENT
            // symbol may THEMSELVES nest a DataRef on `target`
            // (`other[ target[k] ]`); descend so a buried banding access
            // is not missed.
            r.indices
                .iter()
                .any(|ix| expr_bands_target(ix, target, loop_vars, linked))
        }
        IrExpr::Call { args, .. } => args
            .iter()
            .any(|a| expr_bands_target(a, target, loop_vars, linked)),
    }
}

/// Core predicate: is `r` an indexed access to `target` with an index
/// dim that affinely mentions a partitioned iv? A data-dependent index
/// dim is skipped (opaque ⇒ whole-array, the sound case).
fn indexed_ref_bands_target(
    r: &IndexedRef,
    target: &str,
    loop_vars: &[String],
    linked: &LinkedIR,
) -> bool {
    if r.name != target {
        return false;
    }
    for ix in &r.indices {
        // Data-dependent index ⇒ this dim is opaque ⇒ whole-array
        // broadcast (the SOUND scatter case). Not a banding access.
        if expr_contains_dataref_or_call(ix) {
            continue;
        }
        let mut ivs: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        collect_iter_var_refs(ix, loop_vars, &mut ivs);
        if ivs.iter().any(|iv| iv_is_partitioned(linked, iv)) {
            return true;
        }
    }
    false
}
