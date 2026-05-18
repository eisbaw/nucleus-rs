//! ACFG — the Application Control-Flow Graph.
//!
//! The ACFG is the first IR produced *after* the link step (PRD §5,
//! pipeline diagram). It is the input to subsequent passes:
//!
//! - Sync injection (TASK-0017) — populates [`ACFGNode::Sync`] nodes.
//! - Transfer injection (TASK-0018) — populates [`ACFGNode::Xfer`]
//!   nodes.
//! - Petri-net construction (TASK-0027 area) — consumes the elaborated
//!   ACFG.
//!
//! ## Shape: tree, not graph
//!
//! The ACFG is intentionally a **tree** of [`ACFGNode`]s. There are no
//! explicit back-edges; loops are represented by [`ACFGNode::Repeat`]
//! whose `body` is itself an `ACFGNode`. The back-edge from the end of
//! a loop body to its header is therefore *implicit* in the `Repeat`
//! variant. This matches the PRD's note that back-edges are implicit
//! at end of `Repeat` bodies and keeps every algebraic IR pass
//! (lowering, projection, equivalence checking) a structural recursion
//! rather than a graph traversal with cycle detection.
//!
//! Trade-off: if we ever need irreducible control flow (computed
//! goto, exception handlers crossing loops, …) the tree shape will
//! force a redesign. v2 does not have any of these (no conditionals
//! at the algorithm level, PRD §6.2), so the trade is paid for.
//!
//! ## Node kinds
//!
//! - [`ACFGNode::Operation`] — a single basic block, carrying the
//!   kernel firing(s) it covers as a [`DataflowDag`] plus the
//!   worker(s) it runs on.
//! - [`ACFGNode::Repeat`] — a loop, with its iteration variable, the
//!   resolved half-open `i64` range, and its body subtree.
//! - [`ACFGNode::Sequence`] — sequential composition of nodes inside
//!   one scope. Used both at the program top-level and as the body of
//!   a `Repeat` whose source had multiple statements.
//! - [`ACFGNode::Sync`] — barrier inserted by the sync-injection pass
//!   (TASK-0017). Carries the `BTreeSet<WorkerId>` participants. Empty
//!   in a freshly built ACFG (no `Sync` nodes are emitted by
//!   `build_acfg`); the injection pass walks the tree and inserts
//!   them where rules dictate.
//! - [`ACFGNode::Xfer`] — placeholder. Empty payload at M1; populated
//!   by TASK-0018.
//!
//! ## DataflowDag (M1 simplification)
//!
//! The PRD §8.2 and the 2013 thesis (§4.3.6.1 — equivalence by
//! hashing) call for a rich per-block dataflow DAG. v2 M1 ships
//! something deliberately smaller: a `Vec<DataflowEdge>` listing
//! `(data_in[], kernel, data_out)` per firing. This is enough for the
//! sync/transfer-injection passes to see producer/consumer relations
//! inside a block. The richer graph representation (hash-based
//! equivalence, common-subexpression elimination at the dataflow
//! level) is filed as a follow-up (see task self-report and the
//! existing `equivalence-by-hashing` notes in the repo).
//!
//! ## Worker assignment for distributed placements
//!
//! A kernel placed on a worker set (`place k on { w0, w1, w2, w3 }`)
//! is carried in [`Operation::workers`] as a `BTreeSet<WorkerId>`
//! verbatim. The ACFG does *not* replicate the Operation per worker:
//! that projection is a later pass (the per-worker EventList
//! projection, PRD §8.1). Keeping one logical node per algorithm
//! statement makes the tree shape line up 1:1 with the algorithm
//! source structure and keeps the equivalence-by-hashing follow-up
//! tractable.
//!
//! ## ID assignment
//!
//! Kernel/data/worker/iter-var names are turned into opaque `u64` IDs
//! ([`KernelId`], [`DataId`], [`WorkerId`], [`IterVar`]) inside this
//! pass. The mapping is built deterministically from the [`LinkedIR`]
//! input: names are sorted lexicographically, then assigned 0, 1, 2,
//! … in order. Determinism matters because:
//!
//! 1. The ACFG implements `PartialEq` and downstream tests rely on
//!    equality across runs (PRD's regression-test discipline).
//! 2. The hash-based equivalence follow-up needs the same names to
//!    hash to the same IDs across two builds.
//!
//! The name <-> ID mapping is exposed on [`ACFG`] for downstream
//! passes and human-facing diagnostics. Later, when a global name-to-
//! ID assignment pass lands (likely sitting between `link` and
//! `acfg`), this local mapping becomes redundant; for now it is a
//! self-contained convenience.
//!
//! ## What this pass deliberately does NOT do
//!
//! - **Conditionals.** The algorithm sublanguage has none (PRD §6.2,
//!   §6.2.4). Adding `If` to [`ACFGNode`] later is a sum-type
//!   extension; the rest of the IR doesn't have to change.
//! - **Identity copies** (`d <-- e` with a bare DataRef RHS). The
//!   link pass already calls this out as an unexercised corner; the
//!   ACFG pass currently treats such statements as no-ops (no
//!   kernel, no Operation). Filed as a follow-up if a real example
//!   needs it.
//! - **Constant folding beyond what loop bounds require.** Loop
//!   bounds are evaluated to `i64` here because [`Repeat::range`] is
//!   `Range<i64>` (matching [`crate::event::IterTile`]'s element
//!   shape). Index expressions inside a basic block are NOT folded
//!   — they're left as IR expressions so the access-pattern analysis
//!   (a later pass) can inspect them.
//! - **Validation.** This pass assumes its input [`LinkedIR`] is
//!   already validated by `link`. It panics on `Operation`s built
//!   from unplaced kernels, because `link` would have rejected the
//!   program before reaching this pass.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::algo::{AlgoIR, IndexedRef, IrExpr, IrStmt, ResolvedConst};
use crate::event::{DataId, IterTile, IterVar, KernelId, SeqTag, WorkerId};
use crate::link::{LinkedIR, WorkerEntity};
use crate::sched::{NotifyKind, ResolvedPlaceTarget};

// --------------------------------------------------------------------
// Types
// --------------------------------------------------------------------

/// One basic block: a single kernel firing on its assigned worker
/// entity. Carries the per-firing dataflow as a [`DataflowDag`].
///
/// `workers` is a `BTreeSet<WorkerId>` because a kernel can be
/// distributed across several workers
/// (`place k on { w0, w1, w2, w3 }`). For singleton placements
/// (`place k on host`) the set has one element. The projection pass
/// that lowers an Operation to per-worker events decides how to
/// partition the iteration tile across the workers; the ACFG itself
/// stays one node per algorithm statement.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Operation {
    /// Kernel that fires in this basic block.
    pub kernel: KernelId,
    /// Worker entity (one or many workers) the kernel runs on.
    pub workers: std::collections::BTreeSet<WorkerId>,
    /// Dataflow edges within this block. At M1 this is a single
    /// `(data_in[], kernel, data_out)` entry per firing; richer DAGs
    /// (multi-firing fused blocks) are deferred.
    pub dataflow: DataflowDag,
}

/// Dataflow edges inside a basic block. M1: a flat list. Each
/// [`DataflowEdge`] names what flows in, the kernel that consumes
/// it, and what flows out.
///
/// See module docs for why this is a `Vec` and not the richer hash-
/// based DAG of the 2013 thesis §4.3.6.1.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct DataflowDag {
    pub edges: Vec<DataflowEdge>,
}

/// One `(inputs, kernel, output)` entry inside a [`DataflowDag`].
///
/// `data_in` is the unique set of data symbols read by the firing,
/// in the order they appear in the call's argument list. Duplicates
/// are kept — a kernel may read the same `data` twice at different
/// indices (e.g. a stencil kernel reads `img[y-1][x]` and
/// `img[y+1][x]` of the same array). Index expressions are not
/// recorded here at M1; the access pattern is recovered (later) from
/// the source IR.
///
/// `data_out` is `None` for effect statements (the kernel returns
/// `()`) and `Some` for dataflow statements (`d <-- kernel(...)`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct DataflowEdge {
    pub data_in: Vec<DataId>,
    pub kernel: KernelId,
    pub data_out: Option<DataId>,
}

/// Sync placeholder. Populated by the sync-injection pass
/// (TASK-0017). Carries the set of workers that must rendezvous at
/// this barrier. PRD §8.3: a `Sync` event names participants and a
/// [`crate::event::SyncKind`]; the sync-injection pass produces
/// `SyncKind::Barrier` exclusively, so we omit `kind` here and let
/// the later projection pass (ACFG -> per-worker EventList) attach
/// the kind when it materialises [`crate::event::Event::Sync`].
///
/// A `SyncPlaceholder` with fewer than two participants is
/// meaningless (a worker cannot barrier with itself); the injection
/// pass elides such syncs rather than emitting them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct SyncPlaceholder {
    /// Workers that must arrive at this barrier before any proceed.
    /// Stored in a [`std::collections::BTreeSet`] for deterministic
    /// iteration order (matters for codegen determinism downstream).
    pub participants: std::collections::BTreeSet<WorkerId>,
}

/// Notification policy on a transfer. Mirrors the schedule's
/// `notify=event|poll` directive (PRD §6.3.4). `Default` is the
/// backend's choice when the schedule did not specify a notify mode;
/// the codegen-time capability check (TASK-0019+) resolves it against
/// the backend's `capabilities.toml`.
///
/// The variants intentionally do NOT re-use [`crate::sched::NotifyKind`]
/// — we want a third "no preference" state for schedules that didn't
/// state one. Adding `Default` to `NotifyKind` would change the
/// schedule-IR meaning, which we keep distinct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum NotifyMode {
    /// Schedule made no `notify=` choice. Backend picks.
    #[default]
    Default,
    /// Schedule asked for `notify=event` — wait/notify primitive.
    Event,
    /// Schedule asked for `notify=poll` — busy/yield polling.
    Poll,
}

impl From<NotifyKind> for NotifyMode {
    fn from(k: NotifyKind) -> Self {
        match k {
            NotifyKind::Event => NotifyMode::Event,
            NotifyKind::Poll => NotifyMode::Poll,
        }
    }
}

/// Transfer policy resolved from the schedule's `transfer D : ...`
/// directive. Carried verbatim on every [`XferPlaceholder`] for the
/// associated data symbol; downstream passes (Petri net, codegen) read
/// it to decide place capacities and backend lowering.
///
/// Semantics, per PRD §6.3.4:
///
/// - `synchronous = true` means producer blocks until consumer has
///   received (`sync`). False means async — producer returns
///   immediately, consumer waits at use (`async`).
/// - `buffer` is the number of in-flight transfers permitted
///   (`buffer=N`); `1` is the default ("no extra buffering").
///   `>1` enables pipelining.
/// - `notify` is the notification mode (event/poll/default).
///
/// Conflicts between `sync` and `async` in the schedule are not
/// caught at this layer — the linker/lowering passes can deduplicate
/// or reject those; TASK-0018 deliberately leaves capability checking
/// to TASK-0019+ at codegen time when the backend is in hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TransferPolicy {
    /// True iff `sync`. False iff `async`. Default: `true` (sync).
    pub synchronous: bool,
    /// In-flight transfer capacity. Default: `1`. Always `>= 1`.
    pub buffer: u64,
    /// Notification mode chosen by the schedule.
    pub notify: NotifyMode,
}

impl Default for TransferPolicy {
    fn default() -> Self {
        TransferPolicy {
            synchronous: true,
            buffer: 1,
            notify: NotifyMode::Default,
        }
    }
}

/// Xfer placeholder. Populated by the transfer-injection pass
/// (TASK-0018). One `XferPlaceholder` is the projection of *one
/// endpoint* of a matched Push/Wait pair — the [`XferRole`] field
/// tells you which endpoint this is. The matching `seq` and `data`
/// link a `Push` on `src` to a `Wait` on `dst`.
///
/// Why a single struct with a `role` discriminator rather than two
/// `ACFGNode` variants (`Push` / `Wait`): the type already carries
/// `src`/`dst` redundantly; making the variant explicit would not add
/// information and would double the match arms downstream. The roles
/// match one-for-one with `Event::Push` / `Event::Wait` in PRD §8.3.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct XferPlaceholder {
    /// Push (sender side) or Wait (receiver side) of the pair.
    pub role: XferRole,
    /// Source worker (producer side). Same on both endpoints of the
    /// pair so the receiver knows where the data is coming from.
    pub src: WorkerId,
    /// Destination worker (consumer side). Same on both endpoints.
    pub dst: WorkerId,
    /// Data symbol being transferred.
    pub data: DataId,
    /// Iteration tile of the slice in transit. At M1 this is the
    /// enclosing-loop tile of the consumer (per-point granularity).
    /// Coalescing per-tile bulk transfers is a follow-up.
    pub tile: IterTile,
    /// Unique sequence number matching this Push to its Wait. Two
    /// endpoints of the same pair share `seq`.
    pub seq: SeqTag,
    /// Transfer policy from the schedule directive.
    pub policy: TransferPolicy,
}

/// Push (sender side) vs Wait (receiver side) — the endpoint kind of
/// an [`XferPlaceholder`]. Mirrors `Event::Push` / `Event::Wait` in
/// PRD §8.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum XferRole {
    /// Sender side — produces and emits the data slice.
    Push,
    /// Receiver side — awaits and consumes the data slice.
    Wait,
}

/// One node in the ACFG tree.
///
/// `Sequence` is the only variant that can contain other nodes
/// (besides `Repeat`'s `body`); it represents sequential composition
/// inside a scope.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum ACFGNode {
    /// A basic block: one kernel firing on a worker entity.
    Operation(Operation),
    /// A loop. The body is itself an ACFG subtree. Back-edge from
    /// end-of-body to header is implicit in this variant.
    Repeat {
        iter_var: IterVar,
        range: Range<i64>,
        body: Box<ACFGNode>,
    },
    /// Sequential composition of nodes inside one scope.
    Sequence(Vec<ACFGNode>),
    /// Sync barrier placeholder; populated by TASK-0017.
    Sync(SyncPlaceholder),
    /// Cross-worker transfer placeholder; populated by TASK-0018.
    Xfer(XferPlaceholder),
}

/// Root of the ACFG.
///
/// The `root` field is an [`ACFGNode::Sequence`] (or a single
/// [`ACFGNode::Operation`]/`Repeat` if the program had exactly one
/// top-level statement; we keep `Sequence` even for length-1 input so
/// downstream passes have a uniform top-level shape — see
/// [`build_acfg`] notes).
///
/// The `name_*` maps expose the deterministic name <-> ID mapping
/// computed during construction. Useful for diagnostics and for
/// callers that want to interrogate the ACFG against the original
/// algorithm names.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ACFG {
    pub root: ACFGNode,
    /// Kernel name -> KernelId, sorted lexicographically.
    pub name_kernels: BTreeMap<String, KernelId>,
    /// Data symbol name -> DataId, sorted lexicographically.
    pub name_data: BTreeMap<String, DataId>,
    /// Worker name -> WorkerId, sorted lexicographically.
    pub name_workers: BTreeMap<String, WorkerId>,
    /// Iter-var name -> IterVar, sorted lexicographically. Loop
    /// variables share one namespace per the algorithm (PRD §6.2.3);
    /// distinct loop vars get distinct IDs.
    pub name_iter_vars: BTreeMap<String, IterVar>,
    /// IDs of `Repeat::iter_var`s that are *inner* (intra-tile) loops
    /// synthesised by [`crate::passes::block_transform`]. The
    /// transfer-injection pass consults this set to hoist Push/Wait
    /// placeholders out of intra-tile loops up to per-tile
    /// granularity (TASK-0143). Empty for ACFGs built directly from
    /// [`build_acfg`] (i.e. before block-transform has run) and for
    /// programs whose schedule carries no `block=` directive.
    ///
    /// Why a sidecar set instead of a flag on [`ACFGNode::Repeat`]:
    /// keeping the variant payload stable means every existing
    /// pattern match on `Repeat { iter_var, range, body }` keeps
    /// compiling unchanged. The cost is a small lookup at the
    /// hoisting site, which is hot only for blocked schedules.
    #[cfg_attr(feature = "serde", serde(default))]
    pub inner_block_iter_vars: BTreeSet<IterVar>,
}

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
/// Panics if a loop bound cannot be evaluated to an `i64` constant.
/// This is also a tighter invariant than the algorithm IR currently
/// enforces; if a real example trips it, we tighten the lowering
/// pass to reject non-const bounds rather than carry the failure
/// here. Filed as a follow-up.
pub fn build_acfg(linked: &LinkedIR) -> ACFG {
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

    let root_nodes = build_seq(&linked.algo.stmts, &ctx);
    let root = ACFGNode::Sequence(root_nodes);

    ACFG {
        root,
        name_kernels,
        name_data,
        name_workers,
        name_iter_vars,
        inner_block_iter_vars: BTreeSet::new(),
    }
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
fn build_seq(stmts: &[IrStmt], ctx: &BuildCtx<'_>) -> Vec<ACFGNode> {
    let mut out = Vec::with_capacity(stmts.len());
    for s in stmts {
        if let Some(node) = build_stmt(s, ctx) {
            out.push(node);
        }
    }
    out
}

fn build_stmt(stmt: &IrStmt, ctx: &BuildCtx<'_>) -> Option<ACFGNode> {
    match stmt {
        IrStmt::Dataflow { lhs, rhs } => build_dataflow(lhs, rhs, ctx),
        IrStmt::Effect { callee, args } => Some(build_effect(callee, args, ctx)),
        IrStmt::For { var, lo, hi, body } => {
            let iter_var = ctx
                .name_iter_vars
                .get(var)
                .copied()
                .expect("iter var collected during pre-pass");
            let lo_v = eval_const(lo, &ctx.algo.consts)
                .unwrap_or_else(|| panic!("loop lower bound of `{var}` is not a const i64"));
            let hi_v = eval_const(hi, &ctx.algo.consts)
                .unwrap_or_else(|| panic!("loop upper bound of `{var}` is not a const i64"));
            let body_nodes = build_seq(body, ctx);
            // A `Repeat` body is a single ACFGNode. If the body has
            // one statement we still wrap in Sequence for uniform
            // top-level shape downstream; cheap and consistent.
            let body_node = ACFGNode::Sequence(body_nodes);
            Some(ACFGNode::Repeat {
                iter_var,
                range: lo_v..hi_v,
                body: Box::new(body_node),
            })
        }
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
            let data_in = collect_dataref_names(args, ctx.name_data);
            let data_out = ctx.name_data.get(&lhs.name).copied();
            // `data_out` is None only if the LHS isn't a declared
            // data symbol; the lowering pass rejects that (AlgoIR
            // LowerError::AssignmentTargetNotData), so it's safe to
            // expect.
            let data_out = Some(data_out.expect("dataflow LHS must be a declared data symbol"));
            Some(ACFGNode::Operation(Operation {
                kernel: kernel_id,
                workers,
                dataflow: DataflowDag {
                    edges: vec![DataflowEdge {
                        data_in,
                        kernel: kernel_id,
                        data_out,
                    }],
                },
            }))
        }
        // Identity copy or pure-expression RHS: skipped at M1.
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
    let data_in = collect_dataref_names(args, ctx.name_data);
    ACFGNode::Operation(Operation {
        kernel: kernel_id,
        workers,
        dataflow: DataflowDag {
            edges: vec![DataflowEdge {
                data_in,
                kernel: kernel_id,
                data_out: None,
            }],
        },
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

/// Recursively walk an argument list and pull out every `DataRef`'s
/// data name as a [`DataId`], in argument order. Duplicates kept
/// (see [`DataflowEdge::data_in`] doc).
///
/// Index expressions inside a `DataRef` are NOT recursed into for
/// further DataRefs — the algorithm grammar disallows data
/// references in indices (indices are integer expressions over
/// consts and iter vars). Walking would be a no-op; we skip it for
/// brevity.
fn collect_dataref_names(args: &[IrExpr], name_data: &BTreeMap<String, DataId>) -> Vec<DataId> {
    let mut out = Vec::new();
    for a in args {
        collect_dataref_names_expr(a, name_data, &mut out);
    }
    out
}

fn collect_dataref_names_expr(
    e: &IrExpr,
    name_data: &BTreeMap<String, DataId>,
    out: &mut Vec<DataId>,
) {
    match e {
        IrExpr::DataRef(IndexedRef { name, .. }) => {
            if let Some(id) = name_data.get(name) {
                out.push(*id);
            }
        }
        IrExpr::Call { args, .. } => {
            for a in args {
                collect_dataref_names_expr(a, name_data, out);
            }
        }
        IrExpr::Neg(inner) => collect_dataref_names_expr(inner, name_data, out),
        IrExpr::BinOp(_, l, r) => {
            collect_dataref_names_expr(l, name_data, out);
            collect_dataref_names_expr(r, name_data, out);
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
fn eval_const(e: &IrExpr, consts: &BTreeMap<String, ResolvedConst>) -> Option<i64> {
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

// --------------------------------------------------------------------
// Convenience inspection helpers
// --------------------------------------------------------------------

impl ACFGNode {
    /// Count the number of `Operation` nodes in this subtree.
    /// Used by tests to assert structural properties without
    /// snapshotting the whole tree.
    pub fn count_operations(&self) -> usize {
        match self {
            ACFGNode::Operation(_) => 1,
            ACFGNode::Repeat { body, .. } => body.count_operations(),
            ACFGNode::Sequence(children) => children.iter().map(ACFGNode::count_operations).sum(),
            ACFGNode::Sync(_) | ACFGNode::Xfer(_) => 0,
        }
    }

    /// Count the number of `Repeat` (loop) nodes in this subtree.
    pub fn count_repeats(&self) -> usize {
        match self {
            ACFGNode::Repeat { body, .. } => 1 + body.count_repeats(),
            ACFGNode::Sequence(children) => children.iter().map(ACFGNode::count_repeats).sum(),
            ACFGNode::Operation(_) | ACFGNode::Sync(_) | ACFGNode::Xfer(_) => 0,
        }
    }

    /// Maximum nesting depth of `Repeat` nodes. A program with no
    /// loops returns 0; one loop -> 1; nested loops -> 2; etc.
    pub fn max_repeat_depth(&self) -> usize {
        match self {
            ACFGNode::Repeat { body, .. } => 1 + body.max_repeat_depth(),
            ACFGNode::Sequence(children) => children
                .iter()
                .map(ACFGNode::max_repeat_depth)
                .max()
                .unwrap_or(0),
            ACFGNode::Operation(_) | ACFGNode::Sync(_) | ACFGNode::Xfer(_) => 0,
        }
    }
}

impl ACFG {
    /// Convenience: number of `Operation` nodes in the whole ACFG.
    pub fn operation_count(&self) -> usize {
        self.root.count_operations()
    }

    /// Convenience: number of `Repeat` nodes in the whole ACFG.
    pub fn repeat_count(&self) -> usize {
        self.root.count_repeats()
    }

    /// Convenience: maximum loop-nesting depth.
    pub fn max_repeat_depth(&self) -> usize {
        self.root.max_repeat_depth()
    }
}
