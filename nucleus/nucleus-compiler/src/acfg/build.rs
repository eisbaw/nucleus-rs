//! ACFG construction: the [`build_acfg`] entry point + its internal
//! helpers. Consumes a [`LinkedIR`] (the post-link IR pair) and emits
//! an [`super::ACFG`] tree per the determinism + non-goal contract
//! documented at [`super`].
//!
//! Every type used in a public signature here ([`super::ACFG`],
//! [`super::ACFGNode`], [`super::Operation`], [`super::DataflowDag`],
//! [`super::DataflowEdge`], [`super::DataAccess`], [`super::BuildAcfgError`])
//! is defined in a sibling sub-module — see [`super::types`] and
//! [`super::errors`].

use std::collections::{BTreeMap, BTreeSet};

use crate::algo::{AlgoIR, IndexedRef, IrExpr, IrStmt, ResolvedConst};
use crate::event::{ArgBinding, DataId, DataSlice, IterVar, KernelId, WorkerId};
use crate::link::{LinkedIR, WorkerEntity};
use crate::sched::ResolvedPlaceTarget;

use super::{
    ACFGNode, BuildAcfgError, DataAccess, DataflowDag, DataflowEdge, LoopBoundEnd, Operation, ACFG,
};

// --------------------------------------------------------------------
// Entry point
// --------------------------------------------------------------------

/// Build the ACFG from a linked algorithm + schedule pair.
///
/// Panics if a kernel referenced in the algorithm has no placement in
/// `linked.placements`. This should be impossible — `link` enforces
/// every kernel has a placement (PRD §6.3.2) — so a panic here is a
/// linker-pass invariant violation, not a user-facing error.
///
/// Returns [`BuildAcfgError::NonConstLoopBound`] if a `for` loop bound
/// cannot be evaluated to an `i64` constant. This is *reachable* from
/// valid (parse/lower/link-accepted) source — a triangular loop such
/// as `for j : 0 .. i` — so it is a typed diagnostic the driver
/// surfaces cleanly, not a panic (TASK-0179; same precedent as
/// `BlockTransformError` / `SidecarError`).
pub fn build_acfg(linked: &LinkedIR) -> Result<ACFG, BuildAcfgError> {
    // -------- Build the deterministic name-to-ID mapping. --------
    //
    // BTreeMap<String, _> iteration is sorted, so collecting into
    // BTreeMap<String, IdNewtype(u64)> with the index from the
    // iteration is reproducible across runs.

    let name_kernels: BTreeMap<String, KernelId> = linked
        .algo
        .kernels
        .keys()
        .enumerate()
        .map(|(i, name)| (name.clone(), KernelId(i as u64)))
        .collect();

    let name_data: BTreeMap<String, DataId> = linked
        .algo
        .data
        .keys()
        .enumerate()
        .map(|(i, name)| (name.clone(), DataId(i as u64)))
        .collect();

    let name_workers: BTreeMap<String, WorkerId> = linked
        .sched
        .workers
        .keys()
        .enumerate()
        .map(|(i, name)| (name.clone(), WorkerId(i as u64)))
        .collect();

    // Iter-var names: walk every nested `for`. BTreeSet to dedupe and
    // sort, then enumerate.
    let mut iter_var_names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    collect_iter_var_names(&linked.algo.stmts, &mut iter_var_names);
    let name_iter_vars: BTreeMap<String, IterVar> = iter_var_names
        .into_iter()
        .enumerate()
        .map(|(i, name)| (name, IterVar(i as u64)))
        .collect();

    // -------- Build the tree. --------

    let ctx = BuildCtx {
        algo: &linked.algo,
        linked,
        name_kernels: &name_kernels,
        name_data: &name_data,
        name_workers: &name_workers,
        name_iter_vars: &name_iter_vars,
    };

    let root_nodes = build_seq(&linked.algo.stmts, &ctx)?;
    let root = ACFGNode::Sequence(root_nodes);

    Ok(ACFG {
        root,
        name_kernels,
        name_data,
        name_workers,
        name_iter_vars,
        inner_block_iter_vars: BTreeSet::new(),
        // Populated by `passes::partition_workers` (TASK-0212).
        // `build_acfg` is schedule-unaware in this respect; empty
        // means "no per-worker override", so source-range projection
        // applies — identical to pre-TASK-0212 behaviour.
        partition_worker_ranges: BTreeMap::new(),
        // Populated by `passes::transfer_inject` (TASK-0134).
        // `build_acfg` runs before transfer-injection so the map is
        // always empty here; empty means "no pipeline head-start", so
        // every buffer place starts at `initial_marking = 0`.
        pipeline_depth_for_seq: BTreeMap::new(),
        // Populated by `passes::halo_inference` (TASK-0260 Stage 1).
        // `build_acfg` is access-pattern-unaware; empty means
        // "no halo recorded yet" — equivalent to the pre-TASK-0260
        // codegen behaviour, since Stage 2 (transfer_inject extension)
        // is what would observe these.
        halo_widths: BTreeMap::new(),
        // Populated by `passes::reuse_inference` (TASK-0261 Stage 1).
        // `build_acfg` is reuse-directive-unaware; empty means "no
        // reuse loop detected yet" — equivalent to the pre-TASK-0261
        // codegen behaviour, since Stage 2 (backend walker delay-line
        // emit, TASK-0265) is what would observe these.
        reuse_widths: BTreeMap::new(),
        // Populated by `passes::partition_blocks2d` (TASK-0264
        // cycle 113, AC#1). `build_acfg` is schedule-unaware in this
        // respect; empty means "no partition=blocks2d directive", so
        // the pair lookup returns None — the TASK-0289 halo-strip
        // Push/Wait synthesis consumer's only required postcondition.
        partition_pairs: BTreeMap::new(),
        // Populated by `passes::partition_blocks2d` (TASK-0264
        // cycle 113, AC#2). `build_acfg` is worker-count-unaware in
        // this respect; empty means "no grid shape recorded", so the
        // worker -> (row, col) inversion has no entries to read — the
        // TASK-0289 consumer's only required postcondition.
        grid_shape_for_outer_iv: BTreeMap::new(),
    })
}

// --------------------------------------------------------------------
// Internal helpers
// --------------------------------------------------------------------

struct BuildCtx<'a> {
    algo: &'a AlgoIR,
    linked: &'a LinkedIR,
    name_kernels: &'a BTreeMap<String, KernelId>,
    name_data: &'a BTreeMap<String, DataId>,
    name_workers: &'a BTreeMap<String, WorkerId>,
    name_iter_vars: &'a BTreeMap<String, IterVar>,
}

fn collect_iter_var_names(stmts: &[IrStmt], out: &mut std::collections::BTreeSet<String>) {
    for s in stmts {
        if let IrStmt::For { var, body, .. } = s {
            out.insert(var.clone());
            collect_iter_var_names(body, out);
        }
    }
}

/// Build a sequence of ACFGNodes from a flat list of IR statements.
///
/// Each statement becomes at most one node:
/// - `Dataflow { rhs: Call }` -> `Operation`
/// - `Dataflow { rhs: <bare DataRef> }` -> skipped (identity copy;
///    see module docs)
/// - `Effect` -> `Operation`
/// - `For` -> `Repeat`
fn build_seq(stmts: &[IrStmt], ctx: &BuildCtx<'_>) -> Result<Vec<ACFGNode>, BuildAcfgError> {
    let mut out = Vec::with_capacity(stmts.len());
    for s in stmts {
        if let Some(node) = build_stmt(s, ctx)? {
            out.push(node);
        }
    }
    Ok(out)
}

fn build_stmt(stmt: &IrStmt, ctx: &BuildCtx<'_>) -> Result<Option<ACFGNode>, BuildAcfgError> {
    match stmt {
        IrStmt::Dataflow { lhs, rhs } => Ok(build_dataflow(lhs, rhs, ctx)),
        IrStmt::Effect { callee, args } => Ok(Some(build_effect(callee, args, ctx))),
        IrStmt::For { var, lo, hi, body } => {
            let iter_var = ctx
                .name_iter_vars
                .get(var)
                .copied()
                .expect("iter var collected during pre-pass");
            // A non-const loop bound is *diagnosable user input*: the
            // algorithm grammar admits an enclosing iter var in a bound
            // (`for j : 0 .. i`), which lowers/links fine but cannot be
            // folded here. Typed error, not a panic (TASK-0179).
            let lo_v = eval_const(lo, &ctx.algo.consts).ok_or_else(|| {
                BuildAcfgError::NonConstLoopBound {
                    var: var.clone(),
                    end: LoopBoundEnd::Lower,
                    expr: lo.clone(),
                }
            })?;
            let hi_v = eval_const(hi, &ctx.algo.consts).ok_or_else(|| {
                BuildAcfgError::NonConstLoopBound {
                    var: var.clone(),
                    end: LoopBoundEnd::Upper,
                    expr: hi.clone(),
                }
            })?;
            let body_nodes = build_seq(body, ctx)?;
            // A `Repeat` body is a single ACFGNode. If the body has
            // one statement we still wrap in Sequence for uniform
            // top-level shape downstream; cheap and consistent.
            let body_node = ACFGNode::Sequence(body_nodes);
            Ok(Some(ACFGNode::Repeat {
                iter_var,
                range: lo_v..hi_v,
                body: Box::new(body_node),
                // A source loop needs no absolute-index rebinding —
                // it iterates its real range. `block_transform` may
                // later replace it with a tagged inner nest.
                block_tag: None,
            }))
        }
    }
}

/// Map each *top-level* kernel argument to an [`ArgBinding`], in
/// argument order — the positional per-parameter binding (TASK-0156).
///
/// Classification is by the argument's **top-level** shape, mirroring
/// what a backend pattern-matches on:
///
/// - `DataRef` (`a[i]`, `img_in[y-1][x]`, bare aggregate `img_out`)
///   ⇒ [`ArgBinding::Data`];
/// - `Call` (a nested kernel call, e.g.
///   `denoise(mix2(mic_in[frame], bt_in[frame]))` in example 14)
///   ⇒ [`ArgBinding::Nested`], recursing on its arguments;
/// - anything else — an integer/scalar expression over iter vars,
///   consts (and, in principle, embedded data reads like `a[i]+1`)
///   ⇒ [`ArgBinding::Scalar`], carried verbatim as the [`IrExpr`].
///
/// This is total for **link-valid** IR: every argument shape maps to
/// some `ArgBinding` variant without flattening or rejecting. It is
/// NOT panic-free in the absolute — `bind_arg` `panic!`s on a
/// `DataRef` to an undeclared symbol, which the lowering/link pass
/// rejects upstream (`UnknownIdent`) so it cannot reach here for a
/// link-valid program; the panic is a loud guard on that upstream
/// invariant, not an expected path. Faithfully representing a nested
/// call (rather than
/// flattening or rejecting it here) keeps the EventList contract a
/// mirror of the program; whether a given backend can lower a nested
/// call in argument position is the *backend's* decision
/// (the shared `backend_common::render::fire::render_fire_arg`
/// helper rejects it via `EmitError::UnsupportedFeature` — that
/// rejection stays where it is, not duplicated into ACFG
/// construction). The pre-TASK-0150 code already admitted example 14
/// into an ACFG (its `data_in` recursed into the nested call); the
/// binding preserves that, it does not regress it.
fn build_arg_bindings(args: &[IrExpr], name_data: &BTreeMap<String, DataId>) -> Vec<ArgBinding> {
    args.iter().map(|a| bind_arg(a, name_data)).collect()
}

/// Bind one argument expression. See [`build_arg_bindings`].
fn bind_arg(a: &IrExpr, name_data: &BTreeMap<String, DataId>) -> ArgBinding {
    match a {
        IrExpr::DataRef(IndexedRef { name, indices }) => {
            // A DataRef whose symbol isn't a declared data symbol is
            // a lowering-pass invariant violation (it would have been
            // rejected as UnknownIdent); expect with context.
            let data = *name_data.get(name).unwrap_or_else(|| {
                panic!("kernel argument references `{name}`, not a declared data symbol")
            });
            ArgBinding::Data(DataSlice {
                data,
                indices: indices.clone(),
            })
        }
        IrExpr::Call { callee, args } => ArgBinding::Nested {
            callee: callee.clone(),
            args: args.iter().map(|x| bind_arg(x, name_data)).collect(),
        },
        // Integer / scalar expression (IntLit, Ident, Neg, BinOp).
        // Carried verbatim — inert data at this layer.
        scalar => ArgBinding::Scalar(scalar.clone()),
    }
}

fn build_dataflow(lhs: &IndexedRef, rhs: &IrExpr, ctx: &BuildCtx<'_>) -> Option<ACFGNode> {
    match rhs {
        IrExpr::Call { callee, args } => {
            let kernel_id = ctx
                .name_kernels
                .get(callee)
                .copied()
                .expect("kernel id assigned during pre-pass; link guarantees existence");
            let workers = resolve_worker_set(callee, ctx);
            let data_in_access = collect_dataref_access(args, ctx.name_data);
            let data_in = data_in_access.iter().map(|a| a.data).collect();
            let arg_bindings = build_arg_bindings(args, ctx.name_data);
            let data_out = ctx.name_data.get(&lhs.name).copied();
            // `data_out` is None only if the LHS isn't a declared
            // data symbol; the lowering pass rejects that (AlgoIR
            // LowerError::AssignmentTargetNotData), so it's safe to
            // expect.
            let data_out = Some(data_out.expect("dataflow LHS must be a declared data symbol"));
            // TASK-0150: capture the LHS index expressions verbatim
            // (e.g. the `[y][x]` of `img_out[y][x] <-- blur3(...)`).
            let data_out_access = data_out.map(|d| DataAccess {
                data: d,
                indices: lhs.indices.clone(),
            });
            let edge = DataflowEdge {
                data_in,
                kernel: kernel_id,
                data_out,
                data_in_access,
                data_out_access,
                args: arg_bindings,
            };
            edge.debug_check();
            Some(ACFGNode::Operation(Operation {
                kernel: kernel_id,
                workers,
                dataflow: DataflowDag { edges: vec![edge] },
            }))
        }
        // Identity copy or pure-expression RHS: still skipped here.
        // A kernel-less data-move is not representable as an
        // `Operation` today — `Operation.kernel` / `DataflowEdge.kernel`
        // / `Event::Fire.kernel` are non-optional `KernelId`s and there
        // is no schedule directive mapping a data symbol to a worker set
        // (only `place_data D in REGION`, a memory region not a worker).
        // The LINK layer DOES now record this edge's producer/consumer
        // transitively (link::dataflow::propagate_copy_edges, TASK-0347),
        // so a cross-worker identity copy is caught by the
        // MissingCrossWorkerTransfer existence check; the ACFG/codegen
        // half is filed as TASK-0360 (kernel-optional Operation + the
        // worker-set derivation blocker).
        _ => None,
    }
}

fn build_effect(callee: &str, args: &[IrExpr], ctx: &BuildCtx<'_>) -> ACFGNode {
    let kernel_id = ctx
        .name_kernels
        .get(callee)
        .copied()
        .expect("kernel id assigned during pre-pass");
    let workers = resolve_worker_set(callee, ctx);
    let data_in_access = collect_dataref_access(args, ctx.name_data);
    let data_in = data_in_access.iter().map(|a| a.data).collect();
    let arg_bindings = build_arg_bindings(args, ctx.name_data);
    let edge = DataflowEdge {
        data_in,
        kernel: kernel_id,
        data_out: None,
        data_in_access,
        data_out_access: None,
        args: arg_bindings,
    };
    edge.debug_check();
    ACFGNode::Operation(Operation {
        kernel: kernel_id,
        workers,
        dataflow: DataflowDag { edges: vec![edge] },
    })
}

/// Look up the kernel's placement in the linked IR and project it to
/// a `BTreeSet<WorkerId>` using the local name-to-id map. Panics if
/// the kernel has no placement — `link` rejects that.
fn resolve_worker_set(
    kernel_name: &str,
    ctx: &BuildCtx<'_>,
) -> std::collections::BTreeSet<WorkerId> {
    let placement = ctx.linked.placements.get(kernel_name).unwrap_or_else(|| {
        panic!("kernel `{kernel_name}` has no placement; link should have rejected")
    });
    let entity = match &placement.target {
        ResolvedPlaceTarget::One(w) => {
            let mut s = std::collections::BTreeSet::new();
            s.insert(w.clone());
            WorkerEntity(s)
        }
        ResolvedPlaceTarget::Many(ws) => WorkerEntity(ws.iter().cloned().collect()),
    };
    entity
        .0
        .iter()
        .map(|name| {
            ctx.name_workers
                .get(name)
                .copied()
                .unwrap_or_else(|| panic!("worker `{name}` not in name table"))
        })
        .collect()
}

/// Recursively walk an argument list and pull out every `DataRef`
/// as a [`DataAccess`] (resolved [`DataId`] + verbatim index
/// [`IrExpr`]s), in argument order. Duplicates kept (see
/// [`DataflowEdge::data_in`] doc) — a stencil firing reads e.g.
/// `img[y-1][x]` and `img[y+1][x]` of the same array; both appear,
/// in order, with their distinct index lists (TASK-0150).
///
/// The traversal order is identical to the pre-TASK-0150
/// `collect_dataref_names` (depth-first, argument order, recursing
/// into nested calls/neg/binop), so a caller that maps this to just
/// the `DataId`s gets exactly the old `data_in` vector. That is the
/// single-source-of-truth contract: `data_in` is *derived* from
/// `data_in_access`, never built independently.
///
/// Index expressions inside a `DataRef` are intentionally NOT recursed
/// into for further DataRefs — we keep the index list verbatim.
///
/// NOTE (TASK-0341.03.01 / TASK-0375): both halves of the old rationale
/// here are now stale. (a) The algorithm grammar does NOT disallow data
/// references in indices: a data-dependent (gather) index such as
/// `x[col[k]]` parses (the expression surface admits a nested indexed
/// LValue) and lowers (`lower_index_expr`'s `allow_gather`). (b) "Walking
/// would be a no-op" is therefore also false now: with the gather landed,
/// `collect_dataref_access_expr` matching an `IrExpr::DataRef` pushes ONLY
/// the OUTER array and does not descend into `indices`, so for `x[col[k]]`
/// the inner index array `col` is NOT added to this firing's
/// `data_in_access`. Recursing WOULD now find `col`.
///
/// This non-collection is INERT for single-worker codegen, which emits
/// straight-line from AlgoIR and ignores the ACFG dataflow edges (see
/// memory `project-backend-emits-from-algoir-not-acfg`). It is precisely
/// part of why a DISTRIBUTED gather needs dedicated work: the conservative
/// path must broadcast the whole gathered array, and `col` must reach
/// `data_in`. That broadening (including whether to recurse here) is
/// tracked as TASK-0373; deliberately NOT changed in this docstring-only
/// fix, since altering the walk would perturb `data_in` for shipped
/// programs.
fn collect_dataref_access(
    args: &[IrExpr],
    name_data: &BTreeMap<String, DataId>,
) -> Vec<DataAccess> {
    let mut out = Vec::new();
    for a in args {
        collect_dataref_access_expr(a, name_data, &mut out);
    }
    out
}

fn collect_dataref_access_expr(
    e: &IrExpr,
    name_data: &BTreeMap<String, DataId>,
    out: &mut Vec<DataAccess>,
) {
    match e {
        IrExpr::DataRef(IndexedRef { name, indices }) => {
            if let Some(id) = name_data.get(name) {
                out.push(DataAccess {
                    data: *id,
                    indices: indices.clone(),
                });
            }
        }
        IrExpr::Call { args, .. } => {
            for a in args {
                collect_dataref_access_expr(a, name_data, out);
            }
        }
        IrExpr::Neg(inner) => collect_dataref_access_expr(inner, name_data, out),
        IrExpr::BinOp(_, l, r) => {
            collect_dataref_access_expr(l, name_data, out);
            collect_dataref_access_expr(r, name_data, out);
        }
        IrExpr::IntLit(_) | IrExpr::Ident(_) => {}
    }
}

/// Evaluate an `IrExpr` to an `i64` constant. Returns `None` if the
/// expression contains any non-const construct (DataRef, Call, an
/// `Ident` that isn't a declared const).
///
/// Iteration variables are NOT looked up here — loop bounds in the
/// algorithm grammar are const expressions, and nested-loop bounds
/// that reference an outer iter var would be a parser/lowering bug.
/// If a real example demands iter-var-dependent bounds, the lowering
/// pass tightens; we panic here on `None`.
///
/// `pub(crate)` so the link step can reuse it for TASK-0217's
/// iteration-count check without duplicating the evaluator.
pub(crate) fn eval_const(e: &IrExpr, consts: &BTreeMap<String, ResolvedConst>) -> Option<i64> {
    match e {
        IrExpr::IntLit(v) => Some(*v),
        IrExpr::Ident(name) => consts.get(name).map(|c| c.value),
        IrExpr::Neg(inner) => eval_const(inner, consts).and_then(i64::checked_neg),
        IrExpr::BinOp(op, l, r) => {
            use crate::algo::IrBinOp::*;
            let lv = eval_const(l, consts)?;
            let rv = eval_const(r, consts)?;
            match op {
                Add => lv.checked_add(rv),
                Sub => lv.checked_sub(rv),
                Mul => lv.checked_mul(rv),
                Div => {
                    if rv == 0 {
                        None
                    } else {
                        lv.checked_div(rv)
                    }
                }
                Mod => {
                    if rv == 0 {
                        None
                    } else {
                        lv.checked_rem(rv)
                    }
                }
            }
        }
        IrExpr::Call { .. } | IrExpr::DataRef(_) => None,
    }
}
