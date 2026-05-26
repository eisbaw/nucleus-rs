//! TASK-0134: pipeline-depth vs buffer-capacity constraint, plus
//! TASK-0217's pipeline-depth vs iteration-count constraint.
//!
//! Caught at link time (rather than waiting for the boundedness pass
//! to trip) because the link step has the schedule names in
//! source-friendly form — the diagnostic can name the offending
//! {loop_var, data, depth, buffer} directly. By the time
//! `acfg_to_petri` runs, those names have been resolved to integer
//! IDs and the offending loop iter-var may have been block-transformed
//! into multiple new iter-vars, making a precise user-facing
//! diagnostic harder.

use std::collections::{BTreeMap, BTreeSet};

use super::errors::{LinkError, LinkErrorKind, LinkErrorSource};
use crate::algo::{AlgoIR, IndexedRef, IrExpr, IrStmt};
use crate::sched::{ResolvedLoopOption, ResolvedTransferOption, SchedIR};

/// Append [`LinkErrorKind::PipelineExceedsBuffer`] for each `loop V :
/// pipeline=D` whose body contains a cross-worker Push/Wait pair
/// (both producer kernel and consumer kernel inside the loop) for a
/// data symbol with `transfer DATA : buffer=N` where `D > N`.
///
/// "Both endpoints inside" mirrors specifically the
/// `hoist_invariant_waits` semantic in `transfer_inject`:
/// `hoist_invariant_waits` moves the Wait OUT of the loop body for
/// data symbols not produced inside (e.g. `input` in example 13's
/// pipeline-parallel schedule, where `load_input` lives on host
/// outside the loop). When the Wait is hoisted, the IR-level
/// `pipeline_depth_for_seq` annotation no longer applies (the Xfer's
/// tile no longer contains the pipelined iter-var), so the
/// constraint we are policing here also no longer applies.
///
/// **Same-worker carveout (TASK-0214 — closed):** the link check
/// now mirrors `transfer_inject`'s `src == dst` skip. A `transfer X
/// : buffer=N` directive on a same-worker data symbol is harmless
/// (no Xfer is emitted at runtime by `transfer_inject`) and the
/// pipeline-buffer check should not fire on it either, otherwise it
/// would misdirect the user (the actionable bug, if any, is the
/// placement — not the buffer). `data_is_cross_worker(algo, sched,
/// data_name)` is the new gate: it walks the kernels touching
/// `data_name` in `algo.stmts`, looks up each kernel's placement in
/// `sched.places`, and returns true iff MORE THAN ONE distinct
/// worker is named. An unplaced kernel is conservatively treated
/// as cross-worker so a broken-but-cross-worker schedule still
/// gets the pipeline diagnostic.
///
/// Dual-direction check: producer-and-consumer-both-inside (the
/// pipelined inter-stage case, e.g. `feat1`, `feat2` in example 13).
/// One-side-inside-other-outside cases (e.g. `input` flowing INTO
/// the loop from `load_input`, or `output` flowing OUT of the loop
/// to `save_output`) are NOT policed here: the IR will hoist them
/// and the pipeline-depth annotation will not fire.
///
/// Inputs:
/// - `algo` — needed for the body-data-symbol walk + producer/
///   consumer kernel identity per loop body.
/// - `sched` — for the loop directives, transfer buffer values, and
///   kernel placements (to determine which kernels are cross-worker).
/// - `errors` — appended to (one error per (loop_var, data) violation).
///
/// Determinism: outer iteration is `sched.loops.values()` (BTreeMap by
/// loop_var name); the inner check walks a `BTreeSet<String>`.
pub(super) fn check_pipeline_buffer_constraints(
    algo: &AlgoIR,
    sched: &SchedIR,
    errors: &mut Vec<LinkError>,
) {
    for loop_dir in sched.loops.values() {
        // Find the pipeline depth (if any) on this loop. PRD §6.3.3
        // forbids duplicate option keywords (DuplicateLoopOption), so
        // at most one `Pipeline(_)` survives here; we still iterate
        // defensively so a future grammar relaxation doesn't change
        // the answer.
        let Some(depth) = loop_dir.options.iter().find_map(|opt| match opt {
            ResolvedLoopOption::Pipeline(d) => Some(*d),
            _ => None,
        }) else {
            continue;
        };

        // TASK-0217: pipeline depth D must be <= iteration count of the
        // source `for V : LO .. HI`. With D > iter_count, TASK-0213's
        // path-2 elision drops every Push (there are only iter_count of
        // them, but D > iter_count credits are pre-marked), leaving
        // `D - iter_count` leftover tokens in the buffer place — odd
        // semantics. Reject here so the user sees the precise
        // diagnostic rather than an analysis-net oddity.
        //
        // Iteration count is computed by walking algo.stmts to find the
        // `for V` matching this loop_dir.var and evaluating `hi - lo`
        // via the same const-evaluator the ACFG uses. If either bound
        // is non-const (impossible in v2 grammar — every loop bound is
        // a const expression per PRD §6.2) or the for isn't found
        // (would surface as `UnknownLoop` separately), we skip — this
        // is an independent check that doesn't cascade.
        if let Some(iter_count) = find_loop_iter_count(&algo.stmts, &loop_dir.var, &algo.consts) {
            if (depth as i64) > iter_count {
                // TASK-0099: span from `loop V : pipeline=D` loop-var
                // token — the offending pipeline=D lives on this loop
                // directive.
                errors.push(LinkError::maybe_at(
                    LinkErrorKind::PipelineExceedsIterationCount {
                        loop_var: loop_dir.var.clone(),
                        depth,
                        iteration_count: iter_count,
                    },
                    loop_dir.var_span.clone(),
                    LinkErrorSource::Schedule,
                ));
                // Do NOT continue — if both D > iter_count AND D > buffer,
                // report BOTH. They name different problems with
                // different actionable fixes (raise buffer vs reduce D vs
                // extend the loop).
            }
        }

        // Gather data symbols that are BOTH produced and consumed
        // inside `for VAR : ...`. The intersection is the set of
        // symbols whose Push/Wait pair the IR will keep inside the
        // loop (the IR contract that triggers `initial_marking = D`).
        let mut produced: BTreeSet<String> = BTreeSet::new();
        let mut consumed: BTreeSet<String> = BTreeSet::new();
        collect_data_in_loop(
            &algo.stmts,
            &loop_dir.var,
            false,
            &mut produced,
            &mut consumed,
        );

        // Intersect (deterministic — both are BTreeSets).
        for data_name in produced.intersection(&consumed) {
            let Some(tx) = sched.transfers.get(data_name) else {
                continue;
            };
            // TASK-0214: skip same-worker data symbols. The
            // PipelineExceedsBuffer check polices the IR-level
            // initial_marking that `transfer_inject` would attach to
            // a Push/Wait pair — but `transfer_inject` does NOT emit
            // any Xfer when producer and consumer share a worker
            // (`src == dst` skip). A redundant `transfer X : buffer=N`
            // directive on a same-worker symbol therefore has no
            // runtime correlate, and complaining about pipeline depth
            // vs buffer would misdirect the user (the actual issue, if
            // any, is the placement — not the buffer). Mirror the
            // transfer_inject semantic: only police data symbols whose
            // producer + consumer kernels span more than one worker.
            if !data_is_cross_worker(algo, sched, data_name) {
                continue;
            }
            let buffer = tx
                .options
                .iter()
                .find_map(|opt| match opt {
                    ResolvedTransferOption::Buffer(n) => Some(*n),
                    _ => None,
                })
                // Default buffer is 1 (matches TransferPolicy::default,
                // PRD §6.3.4 row `buffer=N`).
                .unwrap_or(1);
            if depth > buffer {
                // TASK-0099: span from `loop V : pipeline=D` loop-var
                // token. Two source tokens are involved (the loop
                // pipeline=D and the transfer buffer=N); we point at
                // the loop because the depth value is what the link
                // pass primarily complains about, mirroring the
                // diagnostic text ("loop `V` has `pipeline=D` but ...").
                // The transfer span is reachable via `tx.data_span` if
                // a future task wants secondary highlighting.
                errors.push(LinkError::maybe_at(
                    LinkErrorKind::PipelineExceedsBuffer {
                        loop_var: loop_dir.var.clone(),
                        data: data_name.clone(),
                        depth,
                        buffer,
                    },
                    loop_dir.var_span.clone(),
                    LinkErrorSource::Schedule,
                ));
            }
        }
    }
}

/// TASK-0214: return `true` iff `data_name` has kernels on MORE THAN
/// ONE distinct worker (i.e., is genuinely cross-worker). When the
/// producer + consumer kernels of a data symbol all share a single
/// worker placement, the data is same-worker and `transfer_inject`'s
/// `src == dst` skip means no Xfer is emitted at runtime — so any
/// pipeline-vs-buffer check on it would misdirect the user.
///
/// Walks `algo.stmts` to find kernels referencing the data symbol on
/// either side of a dataflow stmt or as an arg, then looks up each
/// kernel's placement in `sched.places`. A kernel WITHOUT a placement
/// is treated as unknown — that's a separate `UnplacedKernel` error
/// the link step already reports; here we conservatively count it as
/// "could be on a different worker" so we don't accidentally squelch
/// the pipeline check on a broken schedule.
fn data_is_cross_worker(algo: &AlgoIR, sched: &SchedIR, data_name: &str) -> bool {
    let mut kernels: BTreeSet<String> = BTreeSet::new();
    collect_kernels_touching_data(&algo.stmts, data_name, &mut kernels);
    if kernels.is_empty() {
        // Unreferenced data — let the outer check proceed; the
        // intersect-of-produced-and-consumed already filtered this.
        return true;
    }
    let mut workers: BTreeSet<String> = BTreeSet::new();
    for kernel in &kernels {
        match sched.places.get(kernel) {
            Some(placement) => match &placement.target {
                crate::sched::ir::ResolvedPlaceTarget::One(w) => {
                    workers.insert(w.clone());
                }
                crate::sched::ir::ResolvedPlaceTarget::Many(ws) => {
                    for w in ws {
                        workers.insert(w.clone());
                    }
                }
            },
            None => {
                // Unplaced kernel — conservatively assume distinct
                // worker (don't squelch the check on a broken sched).
                return true;
            }
        }
    }
    workers.len() > 1
}

/// Walk `stmts` and record every kernel name that touches `data_name`
/// — as Dataflow LHS-producing-kernel (RHS expression is a Call), as a
/// Dataflow RHS DataRef inside a kernel's arg list, or as an Effect
/// arg. Recurses into For-bodies.
fn collect_kernels_touching_data(stmts: &[IrStmt], data_name: &str, out: &mut BTreeSet<String>) {
    for s in stmts {
        match s {
            IrStmt::Dataflow { lhs, rhs } => {
                let touches_lhs = lhs.name == data_name;
                let touches_rhs = expr_touches_data(rhs, data_name);
                if touches_lhs || touches_rhs {
                    // The kernel responsible is the RHS top-level Call,
                    // if any.
                    if let Some(name) = call_callee(rhs) {
                        out.insert(name.to_string());
                    }
                }
            }
            IrStmt::Effect { callee, args } => {
                if args.iter().any(|a| expr_touches_data(a, data_name)) {
                    out.insert(callee.clone());
                }
            }
            IrStmt::For { body, .. } => {
                collect_kernels_touching_data(body, data_name, out);
            }
        }
    }
}

/// True iff `e` references `data_name` directly or transitively.
fn expr_touches_data(e: &IrExpr, data_name: &str) -> bool {
    match e {
        IrExpr::DataRef(r) => r.name == data_name,
        IrExpr::Call { args, .. } => args.iter().any(|a| expr_touches_data(a, data_name)),
        IrExpr::BinOp(_, l, r) => {
            expr_touches_data(l, data_name) || expr_touches_data(r, data_name)
        }
        IrExpr::Neg(inner) => expr_touches_data(inner, data_name),
        IrExpr::IntLit(_) | IrExpr::Ident(_) => false,
    }
}

/// If `e` is a top-level Call, return the callee name. Otherwise None.
fn call_callee(e: &IrExpr) -> Option<&str> {
    match e {
        IrExpr::Call { callee, .. } => Some(callee.as_str()),
        _ => None,
    }
}

/// TASK-0217: find the iteration count for `for VAR : LO .. HI`
/// matching `target_var` anywhere in `stmts` (recursing into nested
/// for bodies). Returns `Some(hi - lo)` when a matching for is found
/// and BOTH bounds evaluate to a const i64, else `None`.
///
/// `None` covers two cases that callers want to skip (not report):
/// - The named loop doesn't exist (UnknownLoop is reported separately
///   by the link step's loop-resolution pass).
/// - A bound contains a non-const construct (impossible in v2 grammar;
///   defensive against a future relaxation).
fn find_loop_iter_count(
    stmts: &[IrStmt],
    target_var: &str,
    consts: &BTreeMap<String, crate::algo::ResolvedConst>,
) -> Option<i64> {
    for s in stmts {
        if let IrStmt::For { var, lo, hi, body } = s {
            if var == target_var {
                let lo_v = crate::acfg::eval_const(lo, consts)?;
                let hi_v = crate::acfg::eval_const(hi, consts)?;
                // Negative range = zero iterations; saturate to 0 so
                // `pipeline=D > 0` always fires the diagnostic.
                return Some((hi_v - lo_v).max(0));
            }
            if let Some(n) = find_loop_iter_count(body, target_var, consts) {
                return Some(n);
            }
        }
    }
    None
}

/// Walk `stmts` and, when inside (or under) `for var : ...`, record
/// data symbols by side:
/// - `produced`: every Dataflow LHS name (a kernel writes the symbol).
/// - `consumed`: every DataRef name in Dataflow RHS or Effect args
///   (a kernel reads the symbol).
///
/// `inside` is the accumulator: once we enter the target `for`, every
/// nested loop body is also "inside" — pipeline depth propagates to
/// every transfer in the loop's transitive body.
fn collect_data_in_loop(
    stmts: &[IrStmt],
    target_var: &str,
    inside: bool,
    produced: &mut BTreeSet<String>,
    consumed: &mut BTreeSet<String>,
) {
    for s in stmts {
        match s {
            IrStmt::Dataflow { lhs, rhs } if inside => {
                produced.insert(lhs.name.clone());
                collect_data_refs(rhs, consumed);
                for idx in &lhs.indices {
                    collect_data_refs(idx, consumed);
                }
            }
            IrStmt::Effect { args, .. } if inside => {
                for a in args {
                    collect_data_refs(a, consumed);
                }
            }
            IrStmt::For { var, body, .. } => {
                let now_inside = inside || var == target_var;
                collect_data_in_loop(body, target_var, now_inside, produced, consumed);
            }
            // Stmt outside any enclosing target loop: skip (we only
            // care about transfers happening *inside* the pipelined
            // loop).
            _ => {}
        }
    }
}

/// Recursively visit an expression and record every `DataRef`'s name.
fn collect_data_refs(e: &IrExpr, out: &mut BTreeSet<String>) {
    match e {
        IrExpr::DataRef(IndexedRef { name, indices }) => {
            out.insert(name.clone());
            for idx in indices {
                collect_data_refs(idx, out);
            }
        }
        IrExpr::Call { args, .. } => {
            for a in args {
                collect_data_refs(a, out);
            }
        }
        IrExpr::Neg(inner) => collect_data_refs(inner, out),
        IrExpr::BinOp(_, l, r) => {
            collect_data_refs(l, out);
            collect_data_refs(r, out);
        }
        IrExpr::IntLit(_) | IrExpr::Ident(_) => {}
    }
}
