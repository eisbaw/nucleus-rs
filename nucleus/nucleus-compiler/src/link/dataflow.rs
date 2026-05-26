//! Dataflow helpers shared by the link-step entry point.
//!
//! Two passes over the algorithm IR:
//!
//! - [`collect_loop_vars`] walks every `for VAR : ...` (including
//!   nested) and returns the set of iteration variables — used to
//!   validate schedule `loop` and `check loop` references.
//! - [`analyse_dataflow`] walks every statement and records, for
//!   each data symbol, the producer worker entity (the kernel on the
//!   RHS of a `D <-- Call(...)`) and the set of consumer worker
//!   entities (kernels that read `D` either as a `Call` argument or
//!   as an `Effect` argument).

use std::collections::{BTreeMap, BTreeSet};

use super::types::WorkerEntity;
use crate::algo::{AlgoIR, IndexedRef, IrExpr, IrStmt};

/// Walk every `for VAR : ...` (including nested) and collect the
/// iteration variable names. Used to validate schedule `loop` and
/// `check loop` references.
pub(super) fn collect_loop_vars(algo: &AlgoIR) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    collect_loop_vars_in_stmts(&algo.stmts, &mut out);
    out
}

fn collect_loop_vars_in_stmts(stmts: &[IrStmt], out: &mut BTreeSet<String>) {
    for s in stmts {
        if let IrStmt::For { var, body, .. } = s {
            out.insert(var.clone());
            collect_loop_vars_in_stmts(body, out);
        }
    }
}

/// Walk every statement and record:
/// - For each `Dataflow { lhs, rhs: Call }`, the producer worker
///   entity of `lhs.name` is the placement of the called kernel.
/// - For each kernel call (in any expression position) or effect
///   statement, every `DataRef` argument's name becomes a consumer
///   of that kernel's worker entity.
///
/// Kernels with no placement (because the schedule didn't place
/// them — already an UnplacedKernel error) are skipped: we don't
/// know where they run, so we don't pretend to.
///
/// Identity-copy dataflow (`D <-- E` where the RHS is a bare
/// `DataRef`, not a `Call`) is currently NOT recorded as a producer
/// edge. No example exercises this, and the right semantics for
/// "kernel-less data move" are part of the partition/transfer pass
/// proper (TASK-0117 + TASK-0258 / TASK-0259, the per-partition=
/// consumers — all Done). Filed as a limitation.
pub(super) fn analyse_dataflow(
    algo: &AlgoIR,
    kernel_workers: &BTreeMap<String, WorkerEntity>,
) -> (
    BTreeMap<String, WorkerEntity>,
    BTreeMap<String, BTreeSet<WorkerEntity>>,
) {
    let mut producers: BTreeMap<String, WorkerEntity> = BTreeMap::new();
    let mut consumers: BTreeMap<String, BTreeSet<WorkerEntity>> = BTreeMap::new();
    walk_stmts(&algo.stmts, kernel_workers, &mut producers, &mut consumers);
    (producers, consumers)
}

fn walk_stmts(
    stmts: &[IrStmt],
    kernel_workers: &BTreeMap<String, WorkerEntity>,
    producers: &mut BTreeMap<String, WorkerEntity>,
    consumers: &mut BTreeMap<String, BTreeSet<WorkerEntity>>,
) {
    for s in stmts {
        match s {
            IrStmt::Dataflow { lhs, rhs } => {
                match rhs {
                    IrExpr::Call { callee, args } => {
                        if let Some(worker) = kernel_workers.get(callee) {
                            // Producer of lhs.name is this kernel's worker.
                            producers.insert(lhs.name.clone(), worker.clone());
                            // Args read as consumers by the same kernel.
                            for arg in args {
                                collect_dataref_consumers(arg, worker, consumers);
                            }
                            // Indices on lhs may reference iter vars or
                            // consts; data symbols in indices are not
                            // anticipated by the algorithm grammar.
                            // Walking lhs.indices for DataRefs is a no-op
                            // for the current grammar, but defensive:
                            for idx in &lhs.indices {
                                collect_dataref_consumers(idx, worker, consumers);
                            }
                        }
                    }
                    // Identity-copy or arithmetic-only RHS: no kernel,
                    // no recorded producer. See module docstring.
                    other => {
                        // Even with no producer kernel, expressions in
                        // the RHS could refer to data. Without a kernel
                        // we have no worker entity to attribute the
                        // read to, so we cannot record it as a consumer.
                        // Filed as a limitation; not exercised by the
                        // current examples.
                        let _ = other;
                    }
                }
            }
            IrStmt::Effect { callee, args } => {
                if let Some(worker) = kernel_workers.get(callee) {
                    for arg in args {
                        collect_dataref_consumers(arg, worker, consumers);
                    }
                }
            }
            IrStmt::For { body, .. } => {
                walk_stmts(body, kernel_workers, producers, consumers);
            }
        }
    }
}

/// Recursively visit an expression and, for every `DataRef`, record
/// the named data symbol as consumed by `worker`. `Call` expressions
/// inside arbitrary positions also count: the callee's args may
/// reference data, and those reads are attributed to the OUTER call
/// (or here, to `worker`, which is the OUTER call when this helper
/// is invoked).
///
/// Note: a `Call` nested inside an arg expression is semantically
/// possible per the grammar but not exercised by current examples
/// (kernel calls live at statement RHS, not inside other kernel
/// args). When such a case lands, the inner call's worker should
/// arguably be the outer kernel's worker; we walk into args
/// recursively to remain consistent.
fn collect_dataref_consumers(
    e: &IrExpr,
    worker: &WorkerEntity,
    consumers: &mut BTreeMap<String, BTreeSet<WorkerEntity>>,
) {
    match e {
        IrExpr::DataRef(IndexedRef { name, indices }) => {
            consumers
                .entry(name.clone())
                .or_default()
                .insert(worker.clone());
            for idx in indices {
                collect_dataref_consumers(idx, worker, consumers);
            }
        }
        IrExpr::Call { args, .. } => {
            for a in args {
                collect_dataref_consumers(a, worker, consumers);
            }
        }
        IrExpr::Neg(inner) => collect_dataref_consumers(inner, worker, consumers),
        IrExpr::BinOp(_, l, r) => {
            collect_dataref_consumers(l, worker, consumers);
            collect_dataref_consumers(r, worker, consumers);
        }
        IrExpr::IntLit(_) | IrExpr::Ident(_) => {}
    }
}
