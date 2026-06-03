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
///
/// Returns [`BuildAcfgError::KernelLessDataflowRhs`] if a dataflow
/// statement's RHS is not a kernel call (`c <-- a`, `c <-- a + b`,
/// `c <-- 5`). Also reachable from valid source; before TASK-0360 it
/// was a silent drop (a same-worker bare copy compiled to nothing).
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
/// Each statement becomes **exactly one** node ([`build_stmt`] is
/// total — it never silently drops a statement):
/// - `Dataflow { rhs: Call }` -> `Operation`
/// - `Dataflow { rhs: <non-Call> }` -> `Err(KernelLessDataflowRhs)`
///   (a kernel-less data-move is unsupported; see [`build_dataflow`])
/// - `Effect` -> `Operation`
/// - `For` -> `Repeat`
fn build_seq(stmts: &[IrStmt], ctx: &BuildCtx<'_>) -> Result<Vec<ACFGNode>, BuildAcfgError> {
    let mut out = Vec::with_capacity(stmts.len());
    for s in stmts {
        out.push(build_stmt(s, ctx)?);
    }
    Ok(out)
}

/// Map a [`ConstFoldError`] from a loop bound to the right typed
/// [`BuildAcfgError`] (TASK-0398): a genuinely non-const bound stays
/// `NonConstLoopBound`; a constant bound that overflows `i64` or divides
/// by zero becomes `OverflowingLoopBound`, so the diagnostic does not
/// mis-advise "use a constant bound" for a bound that already is one.
fn loop_bound_error(
    e: ConstFoldError,
    var: &str,
    end: LoopBoundEnd,
    expr: &IrExpr,
) -> BuildAcfgError {
    match e {
        ConstFoldError::NotConst => BuildAcfgError::NonConstLoopBound {
            var: var.to_string(),
            end,
            expr: expr.clone(),
        },
        ConstFoldError::Overflow(op) => BuildAcfgError::OverflowingLoopBound {
            var: var.to_string(),
            end,
            expr: expr.clone(),
            detail: format!("arithmetic overflow ({op})"),
        },
        ConstFoldError::DivByZero => BuildAcfgError::OverflowingLoopBound {
            var: var.to_string(),
            end,
            expr: expr.clone(),
            detail: "division by zero".to_string(),
        },
    }
}

/// Lower one IR statement to its single ACFG node. Total: every
/// statement maps to exactly one node, or to a typed `BuildAcfgError`
/// (e.g. a kernel-less dataflow RHS). It deliberately does NOT return
/// `Option` — there is no "silently produce nothing" outcome, which is
/// the silent-drop affordance TASK-0360 removed (a same-worker bare copy
/// used to vanish here). Keep this signature non-optional so a future
/// statement kind cannot reintroduce a silent drop by returning `None`.
fn build_stmt(stmt: &IrStmt, ctx: &BuildCtx<'_>) -> Result<ACFGNode, BuildAcfgError> {
    match stmt {
        IrStmt::Dataflow { lhs, rhs } => build_dataflow(lhs, rhs, ctx),
        IrStmt::Effect { callee, args } => Ok(build_effect(callee, args, ctx)),
        IrStmt::For { var, lo, hi, body } => {
            let iter_var = ctx
                .name_iter_vars
                .get(var)
                .copied()
                .expect("iter var collected during pre-pass");
            // A non-const loop bound is *diagnosable user input*: the
            // algorithm grammar admits an enclosing iter var in a bound
            // (`for j : 0 .. i`), which lowers/links fine but cannot be
            // folded here. Typed error, not a panic (TASK-0179). An
            // overflowing CONSTANT bound is a distinct error from a
            // non-const one (TASK-0398) — `loop_bound_error` routes each.
            let lo_v = try_eval_const(lo, &ctx.algo.consts)
                .map_err(|e| loop_bound_error(e, var, LoopBoundEnd::Lower, lo))?;
            let hi_v = try_eval_const(hi, &ctx.algo.consts)
                .map_err(|e| loop_bound_error(e, var, LoopBoundEnd::Upper, hi))?;
            let body_nodes = build_seq(body, ctx)?;
            // A `Repeat` body is a single ACFGNode. If the body has
            // one statement we still wrap in Sequence for uniform
            // top-level shape downstream; cheap and consistent.
            let body_node = ACFGNode::Sequence(body_nodes);
            Ok(ACFGNode::Repeat {
                iter_var,
                range: lo_v..hi_v,
                body: Box::new(body_node),
                // A source loop needs no absolute-index rebinding —
                // it iterates its real range. `block_transform` may
                // later replace it with a tagged inner nest.
                block_tag: None,
            })
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

fn build_dataflow(
    lhs: &IndexedRef,
    rhs: &IrExpr,
    ctx: &BuildCtx<'_>,
) -> Result<ACFGNode, BuildAcfgError> {
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
            Ok(ACFGNode::Operation(Operation {
                kernel: kernel_id,
                workers,
                dataflow: DataflowDag { edges: vec![edge] },
            }))
        }
        // Identity copy or pure-expression RHS (bare `DataRef`,
        // arithmetic, literal): a kernel-less data-move is NOT
        // representable as an `Operation` today — `Operation.kernel` /
        // `DataflowEdge.kernel` / `Event::Fire.kernel` are non-optional
        // `KernelId`s and there is no schedule directive mapping a data
        // symbol to a worker set (only `place_data D in REGION`, a
        // memory region not a worker). TASK-0360's design slice decided
        // AGAINST the kernel-optional refactor (option c: keep the
        // explicit-kernel surface) and instead makes this path FAIL
        // LOUD: previously this arm returned `None`, silently dropping
        // the statement — a *same-worker* copy compiled to nothing (the
        // LHS array stayed at its allocation default; the LINK-layer
        // `MissingCrossWorkerTransfer` existence check, fed by
        // link::dataflow::propagate_copy_edges (TASK-0347), only catches
        // the *cross-worker* case). The typed error directs the user to
        // the explicit-identity-kernel workaround (15-transpose's
        // `xpose`). The clean kernel-less data-move IR node remains
        // deferred behind TASK-0360's re-open trigger.
        //
        // Layer choice (acfg-build, not lower): mirrors the sibling
        // `NonConstLoopBound` precedent — both are grammar-legal forms
        // the *build* layer cannot represent. The RHS shape is in fact
        // known at `lower` time, so a future move of this check to
        // `lower` (which carries byte-spans for a better diagnostic) is
        // reasonable; it stays here for now to keep the build-layer
        // diagnostics colocated. Caveat: a *cross-worker* bare copy with
        // a declared `transfer` passes link's MissingCrossWorkerTransfer
        // check and is still rejected HERE — declaring the transfer does
        // NOT rescue the form, because the move has no kernel to fire.
        _ => Err(BuildAcfgError::KernelLessDataflowRhs {
            lhs: lhs.name.clone(),
            rhs: rhs.clone(),
        }),
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
/// For every program WITHOUT a data-dependent (gather) index, the
/// traversal order is identical to the pre-TASK-0150
/// `collect_dataref_names` (depth-first, argument order, recursing
/// into nested calls/neg/binop), so a caller that maps this to just
/// the `DataId`s gets exactly the old `data_in` vector. That is the
/// single-source-of-truth contract: `data_in` is *derived* from
/// `data_in_access`, never built independently.
///
/// TASK-0373 caveat: when an index expression IS a nested DataRef (a
/// gather `x[col[k]]`), this function recurses into the index array
/// `col` BEFORE pushing the outer array `x` — INDEX-FIRST. That is a
/// deliberate divergence from `walk_dataref_names` (which is
/// outer-first); it ensures the index array reaches `data_in` so the
/// distributed transfer pass bands it to each worker. The index-first
/// order is NOT what makes the strict-FIFO endpoints agree on wire
/// order — that is handled separately by the producer-order Wait sort
/// in `transfer_inject::build_waits_for_op` (TASK-0389), so any
/// declaration order is FIFO-correct. See `collect_dataref_access_expr`'s
/// body comment. Non-gather programs never hit this branch, so their
/// `data_in` is byte-for-byte unchanged.
///
/// Index expressions inside a `DataRef` ARE recursed into for further
/// DataRefs (TASK-0373) — the outer array's index list is kept
/// verbatim on its own `DataAccess`, AND any nested index array is
/// collected as its OWN `DataAccess`. For a data-dependent gather
/// `x[col[k]]` this records BOTH `x` (indices `[col[k]]`) and `col`
/// (indices `[k]`).
///
/// HISTORY (TASK-0341.03.01 / TASK-0375 / TASK-0373): the gather landed
/// at the algorithm surface (`lower_index_expr`'s `allow_gather`), so an
/// index CAN now be a nested DataRef. Until TASK-0373 this function
/// pushed ONLY the outer array and did not descend into `indices`, so
/// for `x[col[k]]` the inner index array `col` was NOT added to the
/// firing's `data_in_access`. That non-collection is INERT for
/// single-worker codegen (which emits straight-line from AlgoIR and
/// ignores the ACFG dataflow edges — see memory
/// `project-backend-emits-from-algoir-not-acfg`), but BROKE a
/// DISTRIBUTED gather: the conservative transfer path needs `col` in
/// `data_in` to i-band it to each worker (otherwise the worker
/// references an unbound `col` symbol). TASK-0373 recurses here so
/// `col` reaches every worker via the existing axis-mapping filter
/// (its indices are iv-affine, so it i-bands like `val`). The outer
/// array `x` stays whole-array broadcast (its data-dependent dim is
/// marked OPAQUE in `transfer_inject::record_access_per_dim`).
///
/// SHIPPED-PROGRAM INVARIANCE: the recursion is a NO-OP for every
/// non-gather program — no index expression there contains a nested
/// DataRef/Call, so the index-recursion loop pushes nothing and
/// `data_in` is byte-for-byte unchanged (verified by e2e byte-identity
/// on all existing cells under TASK-0373).
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
            // TASK-0373: recurse into the INDEX expressions FIRST so a
            // data-dependent (gather) index array is collected BEFORE
            // the outer array. For `x[col_idx[i][k]]` this records
            // `col_idx` (indices = `[i][k]`) and THEN `x` (indices =
            // `[col_idx[i][k]]`).
            //
            // TRAVERSAL ORDER (index-FIRST) is the data_in / data_out
            // contract: `data_in` is derived from this traversal order
            // (build_acfg maps data_in_access → data_in), so for a gather
            // `x[col_idx[i][k]]` the index array `col_idx` precedes the
            // outer array `x` in `data_in`. This index-first rule is kept
            // for the data-dependency reason below (the index array must
            // be present in `data_in` so the distributed transfer pass
            // bands it to each worker) — it is NOT relied on for FIFO
            // wire ordering.
            //
            // FIFO ORDERING IS NO LONGER COUPLED TO THIS TRAVERSAL ORDER
            // (TASK-0389, resolving the former TASK-0373 limitation): the
            // worker's per-channel Wait order is now SORTED to the host's
            // per-channel Push order (producer-statement rank) inside
            // `transfer_inject::build_waits_for_op` (see the TASK-0389
            // block there and `producer_rank_by_data`). So the two
            // endpoints traverse each strict-FIFO channel
            // (mp-tcp-bufsync, mp-tcp-poll, via `read_msg_expect`) in the
            // SAME order for ANY declaration order — index-first or not.
            // Before TASK-0389 this index-first traversal HAPPENED to
            // make the orderings coincide for `prog.gather.algo.nuc`
            // (where `col_idx` is declared before `x`), but a program
            // that declared the gathered array before its index array
            // (e.g. `prog.gather_revdecl.algo.nuc`) re-introduced the
            // mismatch — fail-LOUD on bufsync/poll. The
            // `build_waits_for_op` producer-order sort removed that
            // coupling; this traversal order is now purely the data_in
            // dependency contract.
            //
            // Collecting the index array also gets `col_idx` into
            // `data_in` so the distributed transfer pass i-bands it to
            // each worker like `val` (its indices are iv-affine
            // `[i][k]`); without it the worker references an unbound
            // symbol. This is a NO-OP for every non-gather shipped
            // program (no index expression contains a nested
            // DataRef/Call there), so `data_in` is unperturbed for them
            // (verified by e2e byte-identity on all existing cells). The
            // outer array `x` stays whole-array broadcast because its
            // sole index dim is data-dependent → marked OPAQUE in
            // `transfer_inject::record_access_per_dim`.
            for ix in indices {
                collect_dataref_access_expr(ix, name_data, out);
            }
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
        // A comparison can appear in a (bool-typed) RHS; its two operands
        // are integer expressions that may read data, so collect accesses
        // from both (TASK-0341.02.01.02 / S2).
        IrExpr::BinOp(_, l, r) | IrExpr::Compare(_, l, r) => {
            collect_dataref_access_expr(l, name_data, out);
            collect_dataref_access_expr(r, name_data, out);
        }
        IrExpr::IntLit(_) | IrExpr::Ident(_) => {}
    }
}

/// Why [`try_eval_const`] could not fold an `IrExpr` to an `i64`.
///
/// Distinguishing these is load-bearing for loop-bound diagnostics
/// (TASK-0398): a non-const bound and an *overflowing* constant bound are
/// different user errors and must surface as different
/// [`BuildAcfgError`] variants. The old `Option`-only `eval_const`
/// collapsed them, so an overflowing constant bound was mis-reported as
/// `NonConstLoopBound` ("use a constant bound" — which the user already
/// did).
#[derive(Debug)]
pub(crate) enum ConstFoldError {
    /// The expression contains a non-const construct — a `DataRef`, a
    /// `Call`, or an `Ident` that is not a declared `const` (e.g. an
    /// enclosing iteration variable). Not a compile-time constant.
    NotConst,
    /// The expression IS a constant but its `i64` arithmetic overflowed;
    /// carries the operation name (`"add"`/`"sub"`/`"mul"`/`"div"`/
    /// `"mod"`/`"negate"`).
    Overflow(String),
    /// The expression IS a constant but contains a division or modulo by
    /// zero.
    DivByZero,
}

/// Evaluate an `IrExpr` to an `i64` constant, distinguishing WHY it
/// could not be folded (see [`ConstFoldError`]).
///
/// Iteration variables are NOT looked up here — only declared `const`s
/// resolve; an enclosing iter var in a bound (`for j : 0 .. i`) yields
/// [`ConstFoldError::NotConst`]. A failure is NOT a panic: callers
/// surface it as a typed [`BuildAcfgError`] (`build_stmt`'s loop-bound
/// handling routes `NotConst` → `NonConstLoopBound` and overflow /
/// div-by-zero → `OverflowingLoopBound`), or, for the link-step
/// iteration-count check, skip via the [`eval_const`] `Option` wrapper.
fn try_eval_const(
    e: &IrExpr,
    consts: &BTreeMap<String, ResolvedConst>,
) -> Result<i64, ConstFoldError> {
    match e {
        IrExpr::IntLit(v) => Ok(*v),
        IrExpr::Ident(name) => consts
            .get(name)
            .map(|c| c.value)
            .ok_or(ConstFoldError::NotConst),
        IrExpr::Neg(inner) => try_eval_const(inner, consts)?
            .checked_neg()
            .ok_or_else(|| ConstFoldError::Overflow("negate".into())),
        IrExpr::BinOp(op, l, r) => {
            use crate::algo::IrBinOp::*;
            let lv = try_eval_const(l, consts)?;
            let rv = try_eval_const(r, consts)?;
            let (res, opname) = match op {
                Add => (lv.checked_add(rv), "add"),
                Sub => (lv.checked_sub(rv), "sub"),
                Mul => (lv.checked_mul(rv), "mul"),
                Div => {
                    if rv == 0 {
                        return Err(ConstFoldError::DivByZero);
                    }
                    (lv.checked_div(rv), "div")
                }
                Mod => {
                    if rv == 0 {
                        return Err(ConstFoldError::DivByZero);
                    }
                    (lv.checked_rem(rv), "mod")
                }
            };
            res.ok_or_else(|| ConstFoldError::Overflow(opname.into()))
        }
        IrExpr::Call { .. } | IrExpr::DataRef(_) => Err(ConstFoldError::NotConst),
        // A comparison is bool-valued, not an integer constant, and cannot
        // appear in a loop-bound / const position (lowering rejects it
        // with ComparisonNotAllowedHere before this runs). `NotConst` is
        // the correct, non-panicking answer (TASK-0341.02.01.02 / S2).
        IrExpr::Compare(..) => Err(ConstFoldError::NotConst),
    }
}

/// Evaluate an `IrExpr` to an `i64` constant, or `None` if it cannot be
/// folded for ANY reason (non-const construct, overflow, or
/// div-by-zero). The reason-erasing wrapper over [`try_eval_const`] for
/// callers that only need value-or-nothing — notably the link step's
/// TASK-0217 iteration-count check, which simply skips a non-foldable
/// bound. Callers that must DISTINGUISH non-const from overflow (the
/// loop-bound diagnostic in `build_stmt`) use [`try_eval_const`].
///
/// `pub(crate)` so the link step can reuse it without duplicating the
/// evaluator.
pub(crate) fn eval_const(e: &IrExpr, consts: &BTreeMap<String, ResolvedConst>) -> Option<i64> {
    try_eval_const(e, consts).ok()
}
