//! Link step: cross-reference resolution between AlgoIR and SchedIR.
//!
//! This is the first compiler pass that sees BOTH IRs in hand. It
//! validates every name the schedule borrows from the algorithm and
//! every coverage obligation the schedule has against the algorithm
//! (every kernel placed, every cross-worker dataflow has a transfer).
//!
//! See PRD §5 (link step in the pipeline diagram), §6.3.2 (every
//! kernel must have exactly one `place`), §6.3.4 (cross-worker
//! transfers must be declared), and §12 (algorithm changes that break
//! schedules must surface here as named errors).
//!
//! Design choices, called out so they aren't surprises later:
//!
//! - **Collect all errors in one pass.** PRD §12 emphasises that
//!   silent or one-at-a-time failure modes for the algorithm-vs-
//!   schedule contract make the two files dishonest with each other.
//!   We walk the schedule once, gather every dangling reference and
//!   every missing-transfer obligation, then return them as a `Vec`.
//!   No fail-fast.
//!
//! - **Distributed placements treated as a single worker entity for
//!   transfer inference.** A kernel `place k on { w0, w1, w2, w3 }`
//!   is, from the perspective of cross-worker transfer, ONE entity:
//!   when producer and consumer are both placed on the same
//!   `{w0..w3}` set, no transfer is required (partition= handles the
//!   per-element decomposition, scheduled later in TASK-0016+). This
//!   matches `13-cnn-inference/batch_parallel.sched.nuc` where
//!   `feat1`/`feat2` move within the `{w0..w3}` set with no transfer
//!   directive, but `input` and `output` (between `host` and the set)
//!   do carry transfer directives.
//!
//! - **Producer/consumer derived from AlgoIR statements + SchedIR
//!   placements.** The producer of data symbol `D` is the placement
//!   of the kernel on the RHS of a `Dataflow { lhs: D, rhs: Call }`.
//!   Consumers are the placements of kernels that read `D` either as
//!   a `Call` argument or as an `Effect` argument. Identity copies
//!   (`D <-- E` where the RHS is a bare `DataRef`, no kernel) are
//!   not in the current examples; we treat them as "no kernel
//!   involved, no producer worker recorded" and file a follow-up.
//!
//! - **No fuzzy-match suggestions for typos.** Errors carry the
//!   offending name, not a "did you mean?". Filed as follow-up.
//!
//! What this pass explicitly DOES NOT do (deferred):
//!
//! - Type-check kernel signatures against call sites (TASK-0088).
//! - Validate `partition=` policies against placement cardinality.
//! - Per-worker slicing of distributed placements (TASK-0016+).
//! - Resolve transfer/notify semantics against backend capability
//!   matrix (TASK-0019).
//! - Detect data symbols that have no producer at all (could be a
//!   genuine bug; not in the spec for this task).

use std::collections::{BTreeMap, BTreeSet};

use crate::algo::{AlgoIR, IndexedRef, IrExpr, IrStmt};
use crate::sched::{ResolvedPlaceTarget, ResolvedPlacement, SchedIR};

// --------------------------------------------------------------------
// Public types
// --------------------------------------------------------------------

/// A worker placement collapsed into a comparable set.
///
/// `One("host")` and `Many({w0, w1})` are both represented as a
/// `BTreeSet<String>`; the size and contents distinguish them. Using a
/// `BTreeSet` means equality is order-independent (so
/// `{w0, w1} == {w1, w0}`) and the type implements `Ord` so it can
/// key error-message maps deterministically.
///
/// Conceptually this is the "worker entity" the link step reasons
/// about. The PRD's distributed-placement note in §6.3.2 says the
/// compiler eventually partitions the iter space across the named
/// workers; until that pass runs (TASK-0016+) we treat the whole set
/// as one identity for transfer-existence purposes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct WorkerEntity(pub BTreeSet<String>);

impl WorkerEntity {
    fn from_target(t: &ResolvedPlaceTarget) -> Self {
        match t {
            ResolvedPlaceTarget::One(w) => {
                let mut s = BTreeSet::new();
                s.insert(w.clone());
                WorkerEntity(s)
            }
            ResolvedPlaceTarget::Many(ws) => WorkerEntity(ws.iter().cloned().collect()),
        }
    }

    /// Stable, human-readable rendering for error messages and
    /// diagnostic maps. `{host}` for singletons, `{w0,w1,w2}` for
    /// sets; deterministic order (BTreeSet iterates sorted).
    pub fn display(&self) -> String {
        let names: Vec<&str> = self.0.iter().map(|s| s.as_str()).collect();
        format!("{{{}}}", names.join(","))
    }
}

/// The result of linking an [`AlgoIR`] and a [`SchedIR`].
///
/// The two source IRs are kept verbatim. Cross-references resolved
/// during linking are exposed as separate maps; downstream passes
/// (ACFG, Petri-net construction) read them rather than re-deriving
/// them from the source IRs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedIR {
    pub algo: AlgoIR,
    pub sched: SchedIR,
    /// For each kernel declared in the algorithm, the placement
    /// directive from the schedule. Keyed by kernel name.
    ///
    /// Invariant after a successful link: this map's keyset equals
    /// `algo.kernels` keyset. (Linking fails if a kernel is unplaced
    /// or if the schedule places a kernel that doesn't exist.)
    pub placements: BTreeMap<String, ResolvedPlacement>,
    /// For each kernel, the resolved worker entity it runs on.
    /// Convenience derivation of [`Self::placements`] — derived once
    /// here so downstream passes don't re-walk targets.
    pub kernel_workers: BTreeMap<String, WorkerEntity>,
    /// For each data symbol that has at least one producer kernel,
    /// the worker entity that produces it. A producer is the kernel
    /// on the RHS of `D <-- Call(...)` in the algorithm.
    ///
    /// Symbols with no producer (e.g. read-only inputs, identity-
    /// copy targets) are omitted. Symbols with multiple producers
    /// across statements: the AlgoIR lowering pass enforces single-
    /// assignment per scope (PRD §6.2.1), so this is unique per
    /// scope, but a `For`-loop body assigning `D[n]` per iteration
    /// is treated as a single producer placement (the body's kernel
    /// placement). We currently record the LAST observed producer
    /// placement — pre-condition: AlgoIR's single-assignment check
    /// has already rejected genuinely-duplicate producers.
    pub data_producers: BTreeMap<String, WorkerEntity>,
    /// For each data symbol read by some kernel, the set of worker
    /// entities that consume it. Multiple distinct consumer entities
    /// are normal (a hub data feeding several workers).
    pub data_consumers: BTreeMap<String, BTreeSet<WorkerEntity>>,
}

/// Errors produced by the link pass.
///
/// Each variant names a single contract violation between the
/// algorithm and schedule. As with [`crate::algo::ir::LowerError`] and
/// [`crate::sched::ir::SchedLowerError`], positions are not tracked
/// yet — when AST spans land (TASK-0086/0090), these variants gain
/// position fields without surface change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkError {
    /// The algorithm declares a kernel that no `place` directive
    /// names. PRD §6.3.2: "Every kernel referenced in the algorithm
    /// must have exactly one `place`. An unplaced kernel is a
    /// compile error."
    UnplacedKernel(String),
    /// The schedule has a `place K on ...` but the algorithm has no
    /// `kernel K` declaration.
    UnknownKernel(String),
    /// The schedule has a `place_data D in ...` but the algorithm
    /// has no `data D` declaration.
    UnknownData(String),
    /// The schedule references a loop variable (via `loop VAR` or
    /// `check loop VAR`) that is not declared as the iteration
    /// variable of any `for VAR : ...` in the algorithm.
    UnknownLoop(String),
    /// The schedule has a `transfer D : ...` but the algorithm has
    /// no `data D` declaration.
    UnknownTransferData(String),
    /// A data symbol flows from one worker entity to a different
    /// worker entity, but no `transfer` directive exists for it.
    /// PRD §6.3.4: "A `transfer` directive that would cross workers
    /// ... must be present. Omitting it is a compile error".
    MissingCrossWorkerTransfer {
        data: String,
        producer_worker: String,
        consumer_worker: String,
    },
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LinkError::UnplacedKernel(name) => write!(
                f,
                "kernel `{name}` is declared in the algorithm but has no `place` directive in the schedule"
            ),
            LinkError::UnknownKernel(name) => write!(
                f,
                "schedule places kernel `{name}` but no such kernel is declared in the algorithm"
            ),
            LinkError::UnknownData(name) => write!(
                f,
                "schedule references data symbol `{name}` in `place_data` but no such data is declared in the algorithm"
            ),
            LinkError::UnknownLoop(name) => write!(
                f,
                "schedule references loop variable `{name}` but no `for {name} : ...` exists in the algorithm"
            ),
            LinkError::UnknownTransferData(name) => write!(
                f,
                "schedule has `transfer {name}` but no such data is declared in the algorithm"
            ),
            LinkError::MissingCrossWorkerTransfer {
                data,
                producer_worker,
                consumer_worker,
            } => write!(
                f,
                "data symbol `{data}` flows from {producer_worker} to {consumer_worker} but the schedule declares no `transfer {data} : ...` directive"
            ),
        }
    }
}

impl std::error::Error for LinkError {}

// --------------------------------------------------------------------
// Entry point
// --------------------------------------------------------------------

/// Resolve schedule references against the algorithm and emit a
/// [`LinkedIR`]. Collects all errors before returning — never fails
/// fast (PRD §12).
///
/// Order of checks within the single pass:
/// 1. `place` directives: name a kernel that doesn't exist (UnknownKernel).
/// 2. `place_data` directives: name a data symbol that doesn't exist
///    (UnknownData).
/// 3. `transfer` directives: name a data symbol that doesn't exist
///    (UnknownTransferData).
/// 4. `loop` and `check loop` directives: name a loop variable not in
///    the algorithm (UnknownLoop).
/// 5. Coverage: every algorithm kernel has a `place` (UnplacedKernel).
/// 6. Cross-worker transfers: every data symbol whose producer and
///    consumer differ has a transfer directive
///    (MissingCrossWorkerTransfer).
///
/// Steps 1–5 can each report many errors. Step 6 depends on the
/// per-kernel placement having a meaningful worker entity, so we only
/// run cross-worker analysis on kernels whose placement IS in the
/// schedule (skipping kernels reported as UnplacedKernel). This is
/// the deliberate "best-effort downstream check" choice: if you
/// haven't placed a kernel, we can't infer where it produces or
/// consumes, so we don't make up workers to chase a follow-on error.
pub fn link(algo: AlgoIR, sched: SchedIR) -> Result<LinkedIR, Vec<LinkError>> {
    let mut errors: Vec<LinkError> = Vec::new();

    // --- 1, 2, 3: name resolution against algorithm declarations ---

    for placement in sched.places.values() {
        if !algo.kernels.contains_key(&placement.kernel) {
            errors.push(LinkError::UnknownKernel(placement.kernel.clone()));
        }
    }

    for pd in sched.place_data.values() {
        if !algo.data.contains_key(&pd.data) {
            errors.push(LinkError::UnknownData(pd.data.clone()));
        }
    }

    for tx in sched.transfers.values() {
        if !algo.data.contains_key(&tx.data) {
            errors.push(LinkError::UnknownTransferData(tx.data.clone()));
        }
    }

    // --- 4: loop variable resolution ---

    let loop_vars = collect_loop_vars(&algo);
    for loop_dir in sched.loops.values() {
        if !loop_vars.contains(&loop_dir.var) {
            errors.push(LinkError::UnknownLoop(loop_dir.var.clone()));
        }
    }
    for check in sched.checks.values() {
        if !loop_vars.contains(&check.var) {
            // Same diagnostic — the `check loop VAR` and `loop VAR`
            // both name the same algorithm-side variable.
            errors.push(LinkError::UnknownLoop(check.var.clone()));
        }
    }

    // --- 5: coverage — every kernel has a place ---

    for kernel_name in algo.kernels.keys() {
        if !sched.places.contains_key(kernel_name) {
            errors.push(LinkError::UnplacedKernel(kernel_name.clone()));
        }
    }

    // --- Build the convenience maps and producer/consumer index ---
    //
    // Even when there are errors above, we still build placements/
    // workers for the kernels we CAN resolve — the cross-worker check
    // below uses them. Kernels with no placement are simply skipped.

    let placements: BTreeMap<String, ResolvedPlacement> = sched
        .places
        .iter()
        .filter(|(k, _)| algo.kernels.contains_key(*k))
        .map(|(k, p)| (k.clone(), p.clone()))
        .collect();

    let kernel_workers: BTreeMap<String, WorkerEntity> = placements
        .iter()
        .map(|(k, p)| (k.clone(), WorkerEntity::from_target(&p.target)))
        .collect();

    let (data_producers, data_consumers) = analyse_dataflow(&algo, &kernel_workers);

    // --- 6: cross-worker transfer existence ---

    for (data, producer) in data_producers.iter() {
        if let Some(consumers) = data_consumers.get(data) {
            for consumer in consumers.iter() {
                if consumer != producer && !sched.transfers.contains_key(data) {
                    errors.push(LinkError::MissingCrossWorkerTransfer {
                        data: data.clone(),
                        producer_worker: producer.display(),
                        consumer_worker: consumer.display(),
                    });
                }
            }
        }
    }

    // Deduplicate identical errors. Possible because two consumers on
    // the same different-entity could each emit the same
    // "MissingCrossWorkerTransfer" if the loop above visited them as
    // separate entries (it won't, BTreeSet collapses; but defensive).
    // Also catches the degenerate "report each kind once" pattern.
    errors.sort_by_key(|e| format!("{e:?}"));
    errors.dedup();

    if errors.is_empty() {
        Ok(LinkedIR {
            algo,
            sched,
            placements,
            kernel_workers,
            data_producers,
            data_consumers,
        })
    } else {
        Err(errors)
    }
}

// --------------------------------------------------------------------
// Internal helpers
// --------------------------------------------------------------------

/// Walk every `for VAR : ...` (including nested) and collect the
/// iteration variable names. Used to validate schedule `loop` and
/// `check loop` references.
fn collect_loop_vars(algo: &AlgoIR) -> BTreeSet<String> {
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
/// proper (TASK-0016+). Filed as a limitation.
fn analyse_dataflow(
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
