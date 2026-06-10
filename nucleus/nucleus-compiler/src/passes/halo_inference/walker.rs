//! Core halo-inference AlgoIR walker.
//!
//! Carved from `halo_inference.rs` (TASK-0460, content-preserving
//! mega-file split). [`infer_halo_widths`] performs the COLLECTING walk
//! (it does not short-circuit on the first error) so the lenient + (B')
//! partition-policy-aware entry points can both decide per-error;
//! [`commit_halo_widths`] folds the inferred map onto the ACFG. The
//! `HaloMap` / `HaloErrorWithScope` type aliases the public signatures
//! reference live in the parent (`super`) module.

use std::collections::BTreeMap;

use crate::acfg::ACFG;
use crate::algo::{IndexedRef, IrExpr, IrStmt, ResolvedConst};
use crate::event::{IterVar, KernelId};
use crate::link::LinkedIR;
use crate::passes::common::{affine_decompose, expr_contains_dataref_or_call};

use super::{HaloErrorWithScope, HaloInferenceError, HaloMap};

/// Core inference: walk `linked.algo.stmts` and return both the
/// populated halo map AND the typed errors raised along the way,
/// PAIRED with two iv-name vectors at the error-push site: the
/// enclosing-loop scope (outermost-first) AND the ivs the failing
/// index expression actually references. The walker is COLLECTING
/// (does not short-circuit on the first error) so the lenient +
/// (B') partition-policy-aware variants can both walk a per-error
/// decision over the full error list. The strict variant short-
/// circuits on the first error at the entry point.
///
/// Both iv-vectors are load-bearing for
/// [`apply_halo_inference_partition_aware`]'s per-variant (B')
/// rule split — see that function's docstring for the per-variant
/// rule split between scope-fallback (data-dependent /
/// link-invariant) and ivs-in-index (precise affine variants).
/// [`apply_halo_inference`] and [`apply_halo_inference_advisory`]
/// strip both iv-vectors at their call sites (they only need the
/// typed error).
pub(super) fn infer_halo_widths(linked: &LinkedIR, acfg: &ACFG) -> (HaloMap, Vec<HaloErrorWithScope>) {
    let ctx = WalkCtx {
        name_kernels: &acfg.name_kernels,
        name_iter_vars: &acfg.name_iter_vars,
        consts: &linked.algo.consts,
    };
    let mut halo: HaloMap = BTreeMap::new();
    let mut errors: Vec<HaloErrorWithScope> = Vec::new();
    let scope: Vec<String> = Vec::new();
    collect_from_stmts(&linked.algo.stmts, &scope, &ctx, &mut halo, &mut errors);
    (halo, errors)
}

/// Destructure-and-rebuild commit. Pre-validation in the caller means
/// no partial-commit hazard; the partition passes share the same
/// commit shape.
pub(super) fn commit_halo_widths(acfg: ACFG, halo: BTreeMap<KernelId, BTreeMap<IterVar, u64>>) -> ACFG {
    let ACFG {
        root,
        name_kernels,
        name_data,
        name_workers,
        name_iter_vars,
        inner_block_iter_vars,
        partition_worker_ranges,
        pipeline_depth_for_seq,
        halo_widths: _existing,
        // TASK-0261: halo_inference does not consult or mutate the
        // reuse-widths sidecar; forward verbatim. reuse_inference runs
        // separately and writes its own field.
        reuse_widths,
        // TASK-0264 cycle 113: halo_inference does not consult or
        // mutate partition_pairs / grid_shape_for_outer_iv (populated
        // by partition_blocks2d, which runs before halo_inference);
        // forward verbatim.
        partition_pairs,
        grid_shape_for_outer_iv,
    } = acfg;

    ACFG {
        root,
        name_kernels,
        name_data,
        name_workers,
        name_iter_vars,
        inner_block_iter_vars,
        partition_worker_ranges,
        pipeline_depth_for_seq,
        halo_widths: halo,
        reuse_widths,
        partition_pairs,
        grid_shape_for_outer_iv,
    }
}

// --------------------------------------------------------------------
// Statement walker
// --------------------------------------------------------------------

/// Read-only joining context threaded through the recursive walker.
///
/// Bundles the immutable tables every recursion frame needs (kernel
/// name→id, iter-var name→id, const folding table) into one borrow.
/// This keeps the walker function signatures short (3 args + the
/// mutable output map) — clippy's `too_many_arguments` cap on 7 would
/// fire otherwise, and the bundle makes the intent ("here is the
/// join + folding context I read") explicit at every callsite.
pub(super) struct WalkCtx<'a> {
    pub(super) name_kernels: &'a BTreeMap<String, KernelId>,
    pub(super) name_iter_vars: &'a BTreeMap<String, IterVar>,
    pub(super) consts: &'a BTreeMap<String, ResolvedConst>,
}

/// Walk a flat list of [`IrStmt`]s with `scope` naming the enclosing
/// iter-vars (outermost-first). Each call shadows / extends `scope`
/// via a clone-and-push (the IR is small; the algorithm is one walk).
pub(super) fn collect_from_stmts(
    stmts: &[IrStmt],
    scope: &[String],
    ctx: &WalkCtx<'_>,
    out: &mut BTreeMap<KernelId, BTreeMap<IterVar, u64>>,
    errors: &mut Vec<HaloErrorWithScope>,
) {
    for s in stmts {
        match s {
            IrStmt::Dataflow { lhs, rhs } => {
                // RHS may be a Call (one or more kernel invocations
                // possibly nested), an IntLit/Ident/Neg/BinOp expression,
                // or a bare DataRef (identity copy). Halo only applies
                // to kernel calls — walk RHS looking for them.
                //
                // TASK-0373 / TASK-0384: a data-dependent WRITE LHS (a
                // scatter, e.g. `histogram[input[i]] <-- inc(...)`) makes
                // this firing a SCATTER. Carry that bit so a
                // `DataDependentStride` READ raised inside a scatter
                // firing is classified by the scatter-soundness rule in
                // `error_is_fatal_under_partition` (advisory for the
                // input-index partition where the target replicates
                // whole-array; fatal for a band-partitioned target),
                // while a pure gather (an AFFINE LHS such as `y[i]`)
                // relaxes to advisory unconditionally.
                let lhs_data_dependent =
                    lhs.indices.iter().any(expr_contains_dataref_or_call);
                visit_expr_for_calls(rhs, lhs_data_dependent, scope, ctx, out, errors);
            }
            IrStmt::Effect { callee, args } => {
                // Effect statements (load/save) have no LHS write index,
                // so they can never be a data-dependent WRITE.
                process_call(callee, false, args, scope, ctx, out, errors);
            }
            IrStmt::For {
                var,
                lo: _,
                hi: _,
                // `until` (epic S1/S4) is inert here: halo inference does
                // NOT read the early-exit break predicate. (The build_acfg
                // reject was LIFTED in TASK-0341.02.01.05.01 — `for..until`
                // now lowers to `ACFGNode::Repeat { break_cond }`; this pass
                // simply ignores the predicate, which is analysis-invisible
                // to halo inference by binding `until: _`.)
                until: _,
                body,
            } => {
                // Push the loop var onto the scope before recursing into
                // the body. A loop variable might shadow a const name in
                // principle (PRD §6.2.3 lets loop vars use one namespace
                // and shadow at their loop), but the lowering pass already
                // rejects shadowing of declared const/data/kernel — we do
                // NOT need to model that ambiguity here.
                let mut next_scope = scope.to_vec();
                next_scope.push(var.clone());
                collect_from_stmts(body, &next_scope, ctx, out, errors);
            }
        }
    }
}

/// Recursively visit an expression looking for kernel calls. When a
/// `Call` is hit, dispatch to [`process_call`]; otherwise recurse into
/// sub-expressions. DataRefs inside a top-level RHS expression are NOT
/// the halo-relevant case (the halo question is about kernel args, not
/// about top-level reads), but a nested call's args are.
fn visit_expr_for_calls(
    e: &IrExpr,
    lhs_data_dependent: bool,
    scope: &[String],
    ctx: &WalkCtx<'_>,
    out: &mut BTreeMap<KernelId, BTreeMap<IterVar, u64>>,
    errors: &mut Vec<HaloErrorWithScope>,
) {
    match e {
        IrExpr::Call { callee, args } => {
            process_call(callee, lhs_data_dependent, args, scope, ctx, out, errors)
        }
        IrExpr::Neg(inner) => {
            visit_expr_for_calls(inner, lhs_data_dependent, scope, ctx, out, errors)
        }
        // A comparison can appear in a (bool-typed) RHS; a nested kernel
        // call may be in either operand, so visit both
        // (TASK-0341.02.01.02 / S2).
        IrExpr::BinOp(_, lhs, rhs) | IrExpr::Compare(_, lhs, rhs) => {
            visit_expr_for_calls(lhs, lhs_data_dependent, scope, ctx, out, errors);
            visit_expr_for_calls(rhs, lhs_data_dependent, scope, ctx, out, errors);
        }
        IrExpr::IntLit(_) | IrExpr::Ident(_) | IrExpr::DataRef(_) => {}
    }
}

/// Per-firing call context threaded into [`visit_arg`]: the kernel id +
/// name being walked, plus TASK-0373's `lhs_data_dependent` bit (true
/// iff the enclosing `<--` statement writes a data-dependent LHS index
/// — a scatter). Bundled into one struct to keep `visit_arg` under the
/// clippy `too_many_arguments` cap (7) after the TASK-0373 addition.
#[derive(Clone, Copy)]
struct CallSite<'a> {
    kid: KernelId,
    callee: &'a str,
    lhs_data_dependent: bool,
}

/// Inspect a kernel call's arguments for halo-relevant `DataRef`
/// patterns and update `out`. Recursively visits nested calls.
///
/// `lhs_data_dependent` (TASK-0373) flags that the enclosing `<--`
/// writes a data-dependent LHS index (a scatter); it propagates into
/// every `DataDependentStride` raised on this firing's reads so the
/// fatality predicate can keep a scatter's RMW read FATAL.
fn process_call(
    callee: &str,
    lhs_data_dependent: bool,
    args: &[IrExpr],
    scope: &[String],
    ctx: &WalkCtx<'_>,
    out: &mut BTreeMap<KernelId, BTreeMap<IterVar, u64>>,
    errors: &mut Vec<HaloErrorWithScope>,
) {
    // Some "calls" are effect-statement load/capture kernels that the
    // ACFG name_kernels DOES include (every algorithm kernel is in the
    // name table). A miss is a link-invariant break — push the error
    // and skip this call's args (the kid is the join key; without it
    // the args have nowhere to land).
    let kid = match ctx.name_kernels.get(callee) {
        Some(k) => *k,
        None => {
            // Link-invariant break: empty iv set + scope kept for
            // diagnostic only. The per-error fatality predicate
            // would never reach this variant in practice (every
            // production callsite checks `name_kernels` before
            // halo_inference runs); the (B') rule's conservative
            // fallback applies if it ever did.
            errors.push((
                HaloInferenceError::UnknownKernelInCall {
                    callee: callee.to_string(),
                },
                scope.to_vec(),
                Vec::new(),
            ));
            return;
        }
    };

    let call_site = CallSite {
        kid,
        callee,
        lhs_data_dependent,
    };
    for arg in args {
        // Walk the arg tree. If we hit a DataRef, classify each of its
        // indices; if we hit a nested Call, recurse into IT to handle its
        // args (the nested call has its own kernel id + its own halo
        // contribution).
        visit_arg(arg, call_site, scope, ctx, out, errors);
    }
}

/// Recursively walk a kernel arg expression. DataRefs become halo
/// classifications; nested Calls recurse via [`process_call`]; other
/// sub-expressions are walked through (a `BinOp` could in principle wrap
/// a DataRef e.g. `k(grid[y] + 1)` — we handle that by descending).
fn visit_arg(
    e: &IrExpr,
    call_site: CallSite<'_>,
    scope: &[String],
    ctx: &WalkCtx<'_>,
    out: &mut BTreeMap<KernelId, BTreeMap<IterVar, u64>>,
    errors: &mut Vec<HaloErrorWithScope>,
) {
    match e {
        IrExpr::DataRef(IndexedRef { name, indices }) => {
            for (ax_idx, idx_expr) in indices.iter().enumerate() {
                let site = IndexSite {
                    kid: call_site.kid,
                    callee: call_site.callee,
                    ref_name: name,
                    ax_idx,
                    lhs_data_dependent: call_site.lhs_data_dependent,
                };
                classify_index(idx_expr, &site, scope, ctx, out, errors);
            }
        }
        IrExpr::Call { callee: c2, args } => {
            // A nested call inherits the enclosing firing's
            // `lhs_data_dependent` bit (the write target is the same
            // `<--` statement regardless of call nesting).
            process_call(
                c2,
                call_site.lhs_data_dependent,
                args,
                scope,
                ctx,
                out,
                errors,
            )
        }
        IrExpr::Neg(inner) => visit_arg(inner, call_site, scope, ctx, out, errors),
        // A comparison's operands may wrap a banding DataRef (`k(a[y] <=
        // b)`); descend into both so a buried banding access is still
        // classified (TASK-0341.02.01.02 / S2).
        IrExpr::BinOp(_, lhs, rhs) | IrExpr::Compare(_, lhs, rhs) => {
            visit_arg(lhs, call_site, scope, ctx, out, errors);
            visit_arg(rhs, call_site, scope, ctx, out, errors);
        }
        IrExpr::IntLit(_) | IrExpr::Ident(_) => {}
    }
}

// --------------------------------------------------------------------
// Affine-stride detector
// --------------------------------------------------------------------

/// Diagnostic site context for one DataRef index inspection. Bundles
/// the (kernel-id, kernel-name, data-symbol, axis) tuple so each
/// `HaloInferenceError` constructor has the payload it needs in one
/// borrow. Keeps `classify_index` under the clippy `too_many_arguments`
/// cap (7).
struct IndexSite<'a> {
    kid: KernelId,
    callee: &'a str,
    ref_name: &'a str,
    ax_idx: usize,
    /// TASK-0373: propagated from [`CallSite`] — `true` iff the
    /// enclosing `<--` writes a data-dependent LHS (a scatter). Stamped
    /// onto any `DataDependentStride` raised here so the fatality
    /// predicate keeps a scatter's RMW read FATAL.
    lhs_data_dependent: bool,
}

/// Classify one index expression against the [`IndexSite`] it sits in.
///
/// The four legal shapes (see module docs):
/// - Pure constant → no entry written.
/// - `iv` alone → no entry written (halo 0).
/// - `iv + b` / `iv - b` / `b + iv` (b folds to integer) → write `|b|`.
/// - Everything else → typed error.
fn classify_index(
    e: &IrExpr,
    site: &IndexSite<'_>,
    scope: &[String],
    ctx: &WalkCtx<'_>,
    out: &mut BTreeMap<KernelId, BTreeMap<IterVar, u64>>,
    errors: &mut Vec<HaloErrorWithScope>,
) {
    // Detect early: DataRef or Call inside the index = data-dependent
    // READ (a gather, e.g. `x[col_idx[i][k]]`). The runtime value of
    // the loaded index determines which cell of the outer array the
    // worker reads, so no affine halo width exists.
    //
    // TASK-0373: this is no longer a hard reject under partition. We
    // push the `DataDependentStride` error (still empty iv set, scope
    // kept for the diagnostic record), stamping `is_scatter_rmw` from
    // the enclosing firing's LHS-write-is-data-dependent bit. The
    // fatality predicate ([`error_is_fatal_under_partition`]) then
    // splits:
    //   - PURE GATHER (`is_scatter_rmw == false`, affine LHS such as
    //     `y[i]`): ADVISORY. The transfer layer broadcasts the WHOLE
    //     gathered array to every worker (the array's data-dependent
    //     dim is marked OPAQUE in
    //     `transfer_inject::record_access_per_dim`), so any runtime
    //     index is in range. See the soundness-coupling doc on
    //     `common::expr_contains_dataref_or_call`.
    //   - SCATTER RMW (`is_scatter_rmw == true`): the SAME data symbol
    //     is read AND written by a data-dependent address (e.g.
    //     `histogram[input[i]] <-- inc(histogram[input[i]])`). The RHS
    //     read reaches here; under partition the scatter is ADVISORY
    //     when the target replicates whole-array (the input-index
    //     partition — TASK-0384 + TASK-0343 element-wise-sum combine)
    //     and FATAL when a partitioned iv affinely indexes the target (a
    //     bin partition). See `scatter_target_replicates_whole_array`.
    if expr_contains_dataref_or_call(e) {
        errors.push((
            HaloInferenceError::DataDependentStride {
                kernel: site.callee.to_string(),
                ref_name: site.ref_name.to_string(),
                ax_idx: site.ax_idx,
                is_scatter_rmw: site.lhs_data_dependent,
            },
            scope.to_vec(),
            Vec::new(),
        ));
        return;
    }

    // Collect every enclosing-iter-var Ident the expression mentions.
    // If none → pure constant, no halo contribution.
    // If two or more distinct iter-vars → reject as MultipleIterVarsInIndex.
    let mut ivs_used: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    collect_iter_var_refs(e, scope, &mut ivs_used);

    if ivs_used.is_empty() {
        // Pure-constant index. Fold it to verify it's a valid integer
        // (a non-foldable purely-const-name index would surface as
        // NonAffineIndex — but with no iter-vars in scope it's "halo 0
        // for nothing", so we accept silently).
        return;
    }

    // The iv set for the failing-index payload is `ivs_used`
    // verbatim — populated lazily as a `Vec<String>` only at each
    // error-push site (and skipped on the success paths).
    if ivs_used.len() > 1 {
        let iv_list: Vec<String> = ivs_used.iter().cloned().collect();
        errors.push((
            HaloInferenceError::MultipleIterVarsInIndex {
                kernel: site.callee.to_string(),
                ref_name: site.ref_name.to_string(),
                ax_idx: site.ax_idx,
                iter_vars: ivs_used.into_iter().collect(),
            },
            scope.to_vec(),
            iv_list,
        ));
        return;
    }

    let iv_name = ivs_used.into_iter().next().expect("len == 1 checked");

    // Recover (coefficient, offset) from the expression. The detector
    // accepts only coefficient +1; -1 and |c| > 1 are rejected.
    let (coeff, offset) = match affine_decompose(e, &iv_name, ctx.consts) {
        Some(pair) => pair,
        None => {
            errors.push((
                HaloInferenceError::NonAffineIndex {
                    kernel: site.callee.to_string(),
                    ref_name: site.ref_name.to_string(),
                    ax_idx: site.ax_idx,
                },
                scope.to_vec(),
                vec![iv_name.clone()],
            ));
            return;
        }
    };

    if coeff != 1 {
        errors.push((
            HaloInferenceError::StridedAccessNotSupported {
                kernel: site.callee.to_string(),
                ref_name: site.ref_name.to_string(),
                ax_idx: site.ax_idx,
                coefficient: coeff,
            },
            scope.to_vec(),
            vec![iv_name.clone()],
        ));
        return;
    }

    let iv = match ctx.name_iter_vars.get(&iv_name) {
        Some(iv) => *iv,
        None => {
            // Defensive — collect_iter_var_refs only emits names that
            // were pushed onto `scope`, and scope is grown from
            // `IrStmt::For { var, ... }`s the link step inserts into
            // name_iter_vars. The variant exists so an inconsistently-
            // constructed `(LinkedIR, ACFG)` pair fails closed with a
            // typed error rather than panicking (cycle-81 architect
            // review F-P1).
            let iv_payload = vec![iv_name.clone()];
            errors.push((
                HaloInferenceError::UnknownLoopVar { var: iv_name },
                scope.to_vec(),
                iv_payload,
            ));
            return;
        }
    };
    let width = offset.unsigned_abs();
    let per_iv = out.entry(site.kid).or_default();
    let entry = per_iv.entry(iv).or_insert(0);
    if width > *entry {
        *entry = width;
    }
}

/// Walk an expression and union all Ident-leaf names that are present in
/// the enclosing iter-var `scope`. Idents that refer to a const or
/// anything else are ignored at this step (they are treated as part of
/// the "constant" contribution and validated later by [`affine_decompose`]
/// when it tries to const-fold).
pub(super) fn collect_iter_var_refs(
    e: &IrExpr,
    scope: &[String],
    out: &mut std::collections::BTreeSet<String>,
) {
    match e {
        IrExpr::Ident(name) => {
            if scope.iter().any(|s| s == name) {
                out.insert(name.clone());
            }
        }
        IrExpr::IntLit(_) => {}
        IrExpr::Neg(inner) => collect_iter_var_refs(inner, scope, out),
        IrExpr::BinOp(_, lhs, rhs) => {
            collect_iter_var_refs(lhs, scope, out);
            collect_iter_var_refs(rhs, scope, out);
        }
        // DataRef / Call inside the index would have already been rejected
        // upstream (classify_index calls expr_contains_dataref_or_call
        // before us); the no-op here is reached only on the
        // already-rejected path during unit tests of this helper.
        IrExpr::DataRef(_) | IrExpr::Call { .. } => {}
        // A comparison is bool-valued and cannot appear in an index
        // position (lowering rejects it); it contributes no iter-var to an
        // affine index. No-op (TASK-0341.02.01.02 / S2).
        IrExpr::Compare(..) => {}
    }
}

// NOTE: `expr_contains_dataref_or_call` was lifted to
// [`crate::passes::common`] under TASK-0373 so halo + transfer_inject
// share ONE data-dependence predicate (the halo relaxation and the
// transfer whole-array broadcast MUST agree on what "data-dependent"
// means, or workers read garbage). It is imported at the top of this
// module; the body is unchanged from the pre-lift private copy.

// NOTE: `affine_decompose`, `expr_mentions`, and `eval_const_int` were
// moved to [`crate::passes::common`] in cycle 82 (TASK-0261 prerequisite)
// so the reuse-inference pass can share one definition. The semantic
// contract is unchanged; only the call site moved. See the `common`
// module docs for the recognised affine shapes.

// --------------------------------------------------------------------
// Unit tests
// --------------------------------------------------------------------

