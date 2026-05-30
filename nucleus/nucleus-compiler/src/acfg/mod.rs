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
//! - [`ACFGNode::Xfer`] — transfer placeholder inserted by the
//!   transfer-injection pass (TASK-0018). Carries an
//!   [`XferPlaceholder`] with the matched-pair endpoint
//!   ([`XferRole`], `seq` tag, `src`/`dst` worker, [`TransferPolicy`]).
//!   Empty in a freshly built ACFG (no `Xfer` nodes are emitted by
//!   `build_acfg`); transfer_inject walks the tree and inserts them
//!   when distributed kernel placements demand cross-worker data
//!   movement.
//!
//! ## DataflowDag (M1 simplification + TASK-0150 index plumbing)
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
//! TASK-0150 enriches each [`DataflowEdge`] with the per-firing
//! **index expressions** recovered from the AlgoIR — `data_in_access`
//! (parallel to `data_in`) and `data_out_access` (parallel to
//! `data_out`), each a [`DataAccess`] carrying the resolved
//! [`DataId`](crate::event::DataId) plus the verbatim
//! [`IrExpr`](crate::algo::ir::IrExpr) index list (e.g.
//! `img_in[y-1][x+1]` ⇒ `indices = [y-1, x+1]`). This is *plumbing
//! only*: this pass now records the access pattern; it does not yet
//! act on it. The two consumers are:
//!
//! - **Per-Fire value bindings (TASK-0156).** The Event contract
//!   needs to know, per firing, which `(DataId, slice)` feeds each
//!   kernel parameter and which it writes.
//! - **Precise per-tile halo synthesis (TASK-0158, coupled to
//!   TASK-0117 distributed placement; the consumer machinery lives
//!   in TASK-0260 / TASK-0263).** `transfer_inject` originally
//!   hoisted whole-symbol transfers by *structural* loop-invariance;
//!   the index expressions enable tightening that to actual
//!   per-tile halo strips. That tightening is **not** done here —
//!   it lives in `transfer_inject` and `halo_inference` (the ACFG
//!   layer just exposes the index expressions verbatim so those
//!   passes can act on them). See `transfer_inject` module docs.
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
//! ([`KernelId`](crate::event::KernelId), [`DataId`](crate::event::DataId),
//! [`WorkerId`](crate::event::WorkerId), [`IterVar`](crate::event::IterVar))
//! inside this
//! pass. The mapping is built deterministically from the [`LinkedIR`](crate::LinkedIR)
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
//!   ACFG pass treats such statements as no-ops (no kernel, no
//!   `Operation`) because a kernel-less Operation is not representable
//!   today — `Operation.kernel` / `DataflowEdge.kernel` /
//!   `Event::Fire.kernel` are non-optional `KernelId`s, and no schedule
//!   directive maps a data symbol to a worker set. (The *link* pass, by
//!   contrast, DOES now record an identity copy's producer/consumer
//!   transitively — `link::dataflow::propagate_copy_edges`, TASK-0347 —
//!   so a cross-worker copy is caught by the MissingCrossWorkerTransfer
//!   check.) The ACFG/codegen half is filed as TASK-0360.
//! - **Constant folding beyond what loop bounds require.** Loop
//!   bounds are evaluated to `i64` here because `Repeat::range` is
//!   `Range<i64>` (matching [`crate::event::IterTile`]'s element
//!   shape). Index expressions inside a basic block are NOT folded
//!   — they're left as IR expressions so the access-pattern analysis
//!   (a later pass) can inspect them.
//! - **Validation.** This pass assumes its input [`LinkedIR`](crate::LinkedIR) is
//!   already validated by `link`. It panics on `Operation`s built
//!   from unplaced kernels, because `link` would have rejected the
//!   program before reaching this pass.

pub mod build;
pub mod errors;
pub mod types;

pub use build::build_acfg;
pub(crate) use build::eval_const;
pub use errors::{BuildAcfgError, LoopBoundEnd};
pub use types::{
    ACFGNode, DataAccess, DataflowDag, DataflowEdge, NotifyMode, Operation, SyncPlaceholder,
    TransferPolicy, XferPlaceholder, XferRole, ACFG,
};
