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
use crate::algo::{walk_dataref_names, AlgoIR, IndexedRef, IrExpr, IrStmt};

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
/// # Identity-copy dataflow (TASK-0347 / reopens TASK-0097)
///
/// An identity-copy dataflow (`D <-- E` where the RHS is a bare
/// `DataRef` / arithmetic expression over data, NOT a `Call`) has no
/// kernel of its own, so there is no `place <kernel> on ...` directive
/// to read a worker entity from. The copy is nonetheless a real
/// value-flow edge: `D` becomes a function of the source data `S` read
/// by `E`. We attribute the copy by **transitive value flow** over the
/// already-collected kernel-driven producer/consumer maps, needing NO
/// new worker concept:
///
/// - `producer[D] := producer[S]` — `D`'s value originates wherever
///   `S` was produced (its last writer).
/// - `consumers[S] ∪= consumers[D]` — wherever `D` is later read, `S`
///   is transitively read through the copy.
///
/// [`propagate_copy_edges`] applies these two rules to a fixpoint so
/// copy *chains* (`B <-- A; C <-- B`) converge. The
/// `MissingCrossWorkerTransfer` existence check in
/// [`super::build`] then sees the resulting cross-worker flow exactly
/// as it would for a kernel-call edge — closing the silent-invisibility
/// gap TASK-0097 filed.
///
/// SCOPE LIMIT (carried to the ACFG/codegen side): this fixes only the
/// *link-layer producer/consumer inference*. `acfg::build::build_dataflow`
/// still skips a bare-`LValue` RHS (no kernel-less `Operation` node is
/// representable today — `Operation.kernel` / `Event::Fire.kernel` are
/// non-optional `KernelId`), so a bare-`LValue` identity copy still
/// emits no codegen. That structural follow-up is TASK-0360.
pub(super) fn analyse_dataflow(
    algo: &AlgoIR,
    kernel_workers: &BTreeMap<String, WorkerEntity>,
) -> (
    BTreeMap<String, WorkerEntity>,
    BTreeMap<String, BTreeSet<WorkerEntity>>,
) {
    let mut producers: BTreeMap<String, WorkerEntity> = BTreeMap::new();
    let mut consumers: BTreeMap<String, BTreeSet<WorkerEntity>> = BTreeMap::new();
    // Identity-copy edges (`lhs <-- {source data symbols}`) recorded
    // for the second-pass transitive propagation below. Source order
    // is preserved so the fixpoint is deterministic.
    let mut copy_edges: Vec<CopyEdge> = Vec::new();
    walk_stmts(
        &algo.stmts,
        kernel_workers,
        &mut producers,
        &mut consumers,
        &mut copy_edges,
    );
    propagate_copy_edges(&copy_edges, &mut producers, &mut consumers);
    (producers, consumers)
}

/// One identity-copy dataflow edge: `dst <-- E`, where `srcs` are the
/// data symbols read by `E` (the RHS), in source order. The kernel-less
/// copy carries no worker entity of its own; [`propagate_copy_edges`]
/// derives its producer/consumer attribution transitively from the
/// kernel-driven maps.
struct CopyEdge {
    dst: String,
    srcs: Vec<String>,
}

/// Propagate identity-copy value flow into the producer/consumer maps
/// to a fixpoint. See [`analyse_dataflow`] for the two rules and why
/// transitive flow is the right model. Iterates until no map changes,
/// bounded by `edges.len() + 1` passes (a copy chain of length N
/// converges in at most N passes; the `+1` is the no-change detection
/// pass). Order-independent and deterministic: `BTreeMap`/`BTreeSet`
/// give a stable iteration order and the rules are monotone (we only
/// ever insert), so the fixpoint is unique regardless of edge order.
///
/// LIMITATION (multi-source RHS): the canonical identity copy `D <-- S`
/// has exactly one source, so Rule 1 (`producer[D] := producer[S]`) is
/// unambiguous. An arithmetic RHS spanning two differently-placed
/// producers (`D <-- A + B`, A on host, B on w0) is genuinely ambiguous
/// — "which worker computes D" is the same kernel-less worker-set
/// question the ACFG/codegen half (TASK-0360) defers — and Rule 1 here
/// records the last source's producer (last `insert` wins). That only
/// feeds the *advisory* `MissingCrossWorkerTransfer` existence check,
/// which over-reports rather than under-reports a needed transfer, so
/// the conservative direction is safe; a precise multi-source policy
/// rides with TASK-0360 when codegen for kernel-less moves lands.
fn propagate_copy_edges(
    edges: &[CopyEdge],
    producers: &mut BTreeMap<String, WorkerEntity>,
    consumers: &mut BTreeMap<String, BTreeSet<WorkerEntity>>,
) {
    if edges.is_empty() {
        return;
    }
    // Bound: a copy chain of N edges propagates a producer N hops in N
    // passes; +1 to observe the no-change steady state. A `loop` with
    // a changed-flag would also work, but the explicit bound makes
    // non-termination structurally impossible (monotone map growth
    // cannot exceed this many distinct insertions in practice).
    //
    // The `converged` guard below is load-bearing for SAFETY, not just
    // termination: if the bound were ever too small the loop would exit
    // with the fixpoint INCOMPLETE, which under-reports producers /
    // consumers — the dangerous direction (a missed cross-worker edge
    // is a silent missing-transfer / data race, not a loud error). The
    // bound is exactly tight under single-assignment (PRD §6.2.1: copy
    // edges form a forest, longest path <= edge count), so a non-
    // convergence means that invariant was violated upstream; surface
    // it as a debug panic rather than silently under-propagating.
    let max_passes = edges.len() + 1;
    let mut converged = false;
    for _ in 0..max_passes {
        let mut changed = false;
        for edge in edges {
            for src in &edge.srcs {
                // Rule 1: dst inherits src's producer (value origin).
                if let Some(src_producer) = producers.get(src).cloned() {
                    match producers.get(&edge.dst) {
                        Some(existing) if *existing == src_producer => {}
                        _ => {
                            producers.insert(edge.dst.clone(), src_producer);
                            changed = true;
                        }
                    }
                }
                // Rule 2: src gains dst's consumers (transitive reads).
                if let Some(dst_consumers) = consumers.get(&edge.dst).cloned() {
                    let entry = consumers.entry(src.clone()).or_default();
                    for c in dst_consumers {
                        if entry.insert(c) {
                            changed = true;
                        }
                    }
                }
            }
        }
        if !changed {
            converged = true;
            break;
        }
    }
    debug_assert!(
        converged,
        "propagate_copy_edges did not reach a fixpoint within {max_passes} passes; \
         the copy-edge graph exceeded the single-assignment forest assumption \
         (PRD §6.2.1) — producers/consumers may be under-propagated"
    );
}

fn walk_stmts(
    stmts: &[IrStmt],
    kernel_workers: &BTreeMap<String, WorkerEntity>,
    producers: &mut BTreeMap<String, WorkerEntity>,
    consumers: &mut BTreeMap<String, BTreeSet<WorkerEntity>>,
    copy_edges: &mut Vec<CopyEdge>,
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
                    // Identity-copy / arithmetic-only RHS: no kernel, so
                    // no direct worker entity. Record the copy edge for
                    // the transitive value-flow propagation pass
                    // (TASK-0347; reopens TASK-0097). The source data
                    // symbols read by the RHS are collected verbatim;
                    // `propagate_copy_edges` attributes producer/consumer
                    // from the kernel-driven maps.
                    other => {
                        let mut srcs = Vec::new();
                        collect_dataref_names(other, &mut srcs);
                        // Index expressions on the LHS may, in principle,
                        // read data (no current example does; the grammar
                        // restricts indices to iter-var/const arithmetic).
                        // Walking them is a no-op today but keeps the edge
                        // honest if the grammar widens.
                        for idx in &lhs.indices {
                            collect_dataref_names(idx, &mut srcs);
                        }
                        copy_edges.push(CopyEdge {
                            dst: lhs.name.clone(),
                            srcs,
                        });
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
                walk_stmts(body, kernel_workers, producers, consumers, copy_edges);
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

/// Recursively visit an expression and push every `DataRef`'s data
/// symbol name onto `out`, in left-to-right source order (duplicates
/// preserved). The worker-less analogue of [`collect_dataref_consumers`]
/// — used by the identity-copy walk, which has no kernel worker entity
/// to attribute reads to and instead records the bare source-symbol
/// list for the transitive [`propagate_copy_edges`] pass (TASK-0347).
///
/// Order AND duplicates are load-bearing here (unlike the set-sink
/// public [`crate::algo::collect_dataref_names`]): the `CopyEdge.srcs`
/// value-flow list is consumed verbatim. This Vec-sink wrapper and the
/// public set-sink wrapper both delegate to the single shared
/// [`crate::algo::walk_dataref_names`] recursion (consolidated
/// TASK-0343.03.01, retiring the former silent-sibling pair
/// `feedback-silent-sibling-defect`); the shared walker visits in
/// left-to-right source order and calls the sink once per occurrence,
/// so `push`ing each preserves order + dups exactly.
fn collect_dataref_names(e: &IrExpr, out: &mut Vec<String>) {
    walk_dataref_names(e, &mut |name| out.push(name.to_string()));
}
