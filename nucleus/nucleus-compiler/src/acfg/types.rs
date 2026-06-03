//! ACFG type definitions: [`Operation`], [`DataflowDag`],
//! [`DataflowEdge`], [`SyncPlaceholder`], [`NotifyMode`],
//! [`TransferPolicy`], [`XferPlaceholder`], [`XferRole`],
//! [`ACFGNode`], [`ACFG`], and the [`DataAccess`] alias of
//! [`crate::event::DataSlice`].
//!
//! Co-located with these data types are their inherent impls — both
//! the small structural-invariant helpers ([`DataflowEdge::new`] +
//! [`DataflowEdge::debug_check`], [`TransferPolicy::default`],
//! `From<NotifyKind> for NotifyMode`) AND the convenience inspection
//! helpers ([`ACFGNode::count_operations`] /
//! [`ACFGNode::count_repeats`] / [`ACFGNode::max_repeat_depth`] +
//! their [`ACFG`] counterparts). The convenience methods were a
//! distinct section in the pre-split file; cycle-178 precedent is to
//! co-locate small inherent impls with their types unless they have
//! cross-sub-module callers, which these do not.
//!
//! See [`super`] for the module-level rationale: PRD pipeline
//! position, IR shape ("tree, not graph"), determinism contract,
//! and the explicit non-goals of this pass.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::event::{
    ArgBinding, BlockTag, DataId, DataSlice, IterTile, IterVar, KernelId, SeqTag, SyncTag, WorkerId,
};
use crate::sched::NotifyKind;
// `IrExpr` carries the `until COND` break predicate on a source
// `for..until` loop (epic S4, TASK-0341.02.01.05.01). It is the same
// node already carried by `ArgBinding::Scalar`, so the serde contract
// (and the determinism gate) already covers it.
use crate::algo::IrExpr;

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

/// A single indexed access to a data symbol inside a firing — the
/// ACFG-level projection of an AlgoIR [`IndexedRef`](crate::algo::ir::IndexedRef) (PRD §6.2.3).
///
/// **This is an alias of [`crate::event::DataSlice`]** — the same
/// struct the presentation-layer Event contract uses (TASK-0156).
/// TASK-0150 originally introduced a separate `acfg::DataAccess`;
/// TASK-0156 needed structurally the *same* thing on `Event::Fire`,
/// so the two were collapsed into one definition (single source of
/// truth — PRD principle: never duplicate state). The name
/// `DataAccess` is kept as the ACFG-facing alias so existing
/// TASK-0150 call sites and the public re-export do not churn.
///
/// `data` is the resolved [`DataId`]; `indices` carries the per-axis
/// AlgoIR [`IrExpr`](crate::algo::ir::IrExpr) index expressions verbatim (e.g.
/// `img_in[y-1][x+1]` ⇒ `[y-1, x+1]`, outer dimension first). A
/// scalar / whole-array read has an empty `indices`.
///
/// The AlgoIR [`IrExpr`](crate::algo::ir::IrExpr) tree is carried directly rather than
/// re-encoded: the index grammar lives once, in `algo::ir`; the
/// expressions are inert data at this layer (no pass folds them).
///
/// `DataAccess` lives *alongside* the bare [`DataflowEdge::data_in`]
/// / [`DataflowEdge::data_out`], not replacing them: `sync_inject`,
/// `transfer_inject`, the pthreads-sync ACFG walk, and
/// `block_transform` all still consume the bare `Vec<DataId>` shape.
pub type DataAccess = DataSlice;

/// One `(inputs, kernel, output)` entry inside a [`DataflowDag`].
///
/// `data_in` is the unique set of data symbols read by the firing,
/// in the order they appear in the call's argument list. Duplicates
/// are kept — a kernel may read the same `data` twice at different
/// indices (e.g. a stencil kernel reads `img[y-1][x]` and
/// `img[y+1][x]` of the same array).
///
/// `data_in_access` is the **index-carrying** parallel of `data_in`:
/// one [`DataAccess`] per read, in the same argument order, with the
/// per-firing index expressions recovered from the AlgoIR
/// (TASK-0150). It is a strict super-set of the information in
/// `data_in` (same `DataId`s, same order, same duplicates) plus the
/// indices. `data_in` is retained for the passes that only need the
/// bare symbol set; new consumers (per-Fire value bindings —
/// TASK-0156; precise per-tile halo synthesis — TASK-0260 /
/// TASK-0263) read `data_in_access`.
///
/// `data_out` is `None` for effect statements (the kernel returns
/// `()`) and `Some` for dataflow statements (`d <-- kernel(...)`).
/// `data_out_access` mirrors it with the LHS index expressions
/// (e.g. the `[y][x]` of `img_out[y][x] <-- blur3(...)`).
///
/// `args` (TASK-0156) is the **positional, per-kernel-parameter**
/// binding: one [`ArgBinding`] per kernel argument, in declared
/// argument order, capturing *every* argument — an indexed data read
/// (`a[i]`, `img_in[y-1][x]`, bare aggregate `img_out`) *or* a scalar
/// arithmetic expression over iter vars / consts. This is a strict
/// super-set of `data_in_access` (which keeps only the data-read
/// args, recursively flattened): `args` is what a backend needs to
/// reconstruct the *kernel call* from the EventList alone, parameter
/// by parameter. `data_in` / `data_in_access` are kept for the
/// passes that only want the data-symbol view (sync/transfer
/// injection, the dataflow producer/consumer analysis).
///
/// Invariant, enforced by [`DataflowEdge::debug_check`] (called at
/// every construction site in `build_acfg` and by
/// [`DataflowEdge::new`]; `debug_assert!`, so zero release cost):
///
/// 1. `data_in == data_in_access.iter().map(|a| a.data)` — same
///    length, order, and duplicate structure;
/// 2. `data_out_access.as_ref().map(|a| a.data) == data_out`.
///
/// `args` is the positional per-parameter binding (one entry per
/// kernel argument, in source order). Its `Data` *leaves* — reached
/// by recursing through `Nested` — line up with the `DataRef` leaves
/// of the call, which is also how `data_in_access` is collected. It
/// is deliberately NOT asserted equal to `data_in_access`: an
/// argument that is arithmetic *on* data (e.g. `k(a + b)`) is carried
/// as a single `Scalar` and not decomposed at this layer, whereas
/// `data_in_access` still flattens the `a`/`b` reads inside it. The
/// two views agree for the DataRef/nested-call argument shapes the v2
/// examples use; the divergence on arithmetic-of-data args is a known
/// modelling limitation owned by TASK-0158, not an invariant break.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct DataflowEdge {
    pub data_in: Vec<DataId>,
    pub kernel: KernelId,
    pub data_out: Option<DataId>,
    /// Index-carrying parallel of `data_in` (TASK-0150). Same length,
    /// order, and duplicate structure as `data_in`.
    #[cfg_attr(feature = "serde", serde(default))]
    pub data_in_access: Vec<DataAccess>,
    /// Index-carrying parallel of `data_out` (TASK-0150). `None` iff
    /// `data_out` is `None`.
    #[cfg_attr(feature = "serde", serde(default))]
    pub data_out_access: Option<DataAccess>,
    /// Positional per-parameter argument binding (TASK-0156). One
    /// entry per kernel argument, in argument order, data reads
    /// *and* scalar expressions alike.
    #[cfg_attr(feature = "serde", serde(default))]
    pub args: Vec<ArgBinding>,
}

impl DataflowEdge {
    /// Build an edge from the bare symbol lists, deriving
    /// index-less [`DataAccess`] entries (empty `indices`).
    ///
    /// This is the constructor for callers that do **not** model
    /// per-firing index expressions — principally synthetic test
    /// fixtures and any pass that fabricates an edge structurally
    /// (e.g. `block_transform`'s test helpers). `build_acfg` does
    /// NOT use this: it has the AlgoIR and populates real indices
    /// directly (TASK-0150). Keeping one constructor here means a
    /// test never has to hand-write the parallel access vectors and
    /// risk them drifting out of sync with `data_in`/`data_out` —
    /// the derivation is done once, in one place.
    pub fn new(data_in: Vec<DataId>, kernel: KernelId, data_out: Option<DataId>) -> Self {
        let data_in_access: Vec<DataAccess> = data_in
            .iter()
            .map(|d| DataAccess {
                data: *d,
                indices: Vec::new(),
            })
            .collect();
        let data_out_access = data_out.map(|d| DataAccess {
            data: d,
            indices: Vec::new(),
        });
        // For a synthetic edge built from bare ids, the positional
        // per-parameter binding is exactly the data reads (no scalar
        // args modelled — these callers don't carry index/scalar
        // info). Derived from `data_in_access` so the
        // single-source-of-truth invariant (args' Data variants
        // project to data_in_access) holds even for test fixtures.
        let args = data_in_access
            .iter()
            .cloned()
            .map(ArgBinding::Data)
            .collect();
        let edge = DataflowEdge {
            data_in,
            kernel,
            data_out,
            data_in_access,
            data_out_access,
            args,
        };
        edge.debug_check();
        edge
    }

    /// `debug_assert!` the cross-field sync invariant documented on
    /// the struct (TASK-0150 / review P1). Zero release cost. Called
    /// by `new` and at every `build_acfg` construction site so a
    /// future pass that mutates one field without its parallel is
    /// caught loudly at the seam rather than mis-codegen'd far away.
    pub fn debug_check(&self) {
        debug_assert!(
            self.data_in.len() == self.data_in_access.len()
                && self
                    .data_in
                    .iter()
                    .zip(&self.data_in_access)
                    .all(|(d, a)| *d == a.data),
            "DataflowEdge invariant: data_in must equal data_in_access \
             projected (got data_in={:?}, data_in_access data={:?})",
            self.data_in,
            self.data_in_access
                .iter()
                .map(|a| a.data)
                .collect::<Vec<_>>(),
        );
        debug_assert!(
            self.data_out_access.as_ref().map(|a| a.data) == self.data_out,
            "DataflowEdge invariant: data_out_access.data must equal \
             data_out (got data_out={:?}, data_out_access.data={:?})",
            self.data_out,
            self.data_out_access.as_ref().map(|a| a.data),
        );
    }
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
///
/// `sync` is the stable cross-worker barrier identity (TASK-0172),
/// the `Sync` analogue of [`XferPlaceholder::seq`]. It is assigned by
/// the sync-injection pass — the site where the *global* barrier
/// structure is visible — monotonically in a deterministic pre-order
/// walk, and threaded verbatim through `petri_to_events` into
/// [`crate::event::Event::Sync`]. One barrier is one `SyncPlaceholder`
/// projected (cloned) into every participant's `EventList`, so all
/// participants carry the same tag; that is what lets disjoint
/// per-worker lists agree on barrier identity without a global walk.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct SyncPlaceholder {
    /// Workers that must arrive at this barrier before any proceed.
    /// Stored in a [`std::collections::BTreeSet`] for deterministic
    /// iteration order (matters for codegen determinism downstream).
    pub participants: std::collections::BTreeSet<WorkerId>,
    /// Stable cross-worker barrier identity. Same value for every
    /// participant of this barrier; distinct between distinct
    /// barriers. Assigned by `inject_syncs`.
    pub sync: SyncTag,
}

/// Notification policy on a transfer. Mirrors the schedule's
/// `notify=event|poll` directive (PRD §6.3.4). `Default` means the
/// schedule stated no notify mode, so it carries no constraint and is
/// always satisfiable — the choice is nominally the backend's, though
/// no tier-1 backend currently specialises on this field (it is
/// carried for completeness but unread at codegen today). The driver's
/// capability gate (`check_schedule_compat`, TASK-0019, Done) validates
/// only an *explicit* `notify=` against the chosen backend's
/// `capabilities.toml`, before codegen.
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
/// to the driver's capability gate (`check_schedule_compat`, TASK-0019,
/// Done), which runs once the backend is in hand (after selection,
/// before codegen).
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
        /// `Some` iff this `Repeat` is a strip-mined *inner* loop
        /// produced by [`crate::passes::block_transform`]; carries the
        /// per-occurrence absolute-index rebinding facts threaded onto
        /// the projected [`crate::event::Event::Loop`] (TASK-0180).
        /// `None` for every source loop (built by [`build_acfg`](crate::acfg::build_acfg)) and
        /// for every synthesised *tile* loop — neither needs
        /// rebinding. serde-default so an old wire payload (no field)
        /// deserialises as `None`.
        ///
        /// Why a payload field and not another per-`IterVar` sidecar
        /// set like [`ACFG::inner_block_iter_vars`]: the rebinding fact
        /// is per-loop-**occurrence**, and `block_transform` reuses ONE
        /// `IterVar` across every strip-mined pass / across the
        /// full+partial split. A per-`IterVar` map (`BTreeSet<IterVar>`
        /// or `BTreeMap<IterVar, _>`) structurally collapses those
        /// occurrences onto one key — exactly the conflation that
        /// caused the 04-prefix-sum/blocked double-count (TASK-0180).
        /// The per-occurrence node is the only carrier that can
        /// distinguish them, so the field lives here despite the
        /// (mechanical) destructuring churn.
        #[cfg_attr(feature = "serde", serde(default))]
        block_tag: Option<BlockTag>,
        /// `Some(cond)` iff this `Repeat` is a source `for..until` loop
        /// (epic S4, TASK-0341.02.01.05.01): the bounded early-exit halt
        /// predicate, carried verbatim from the IR so codegen can emit
        /// the runtime break. `None` for every ordinary fixed-iteration
        /// `for` loop AND for every synthesised tile / partition / inner
        /// loop (those are compiler machinery, never a source convergence
        /// loop, so they carry no break predicate).
        ///
        /// ANALYSIS-INVISIBLE TO THE NET. The Petri unroll
        /// ([`crate::passes::acfg_to_petri`]) MUST ignore this field: a
        /// `for..until` lowers to the SAME bounded structure as a plain
        /// `for` over the cap `range`, and the convergence predicate is
        /// not a control place the Net models. Boundedness is proved on
        /// the full-`range` unroll; any early-exit prefix `0..k`
        /// (`k <= range.len()`) is a sub-trace of that bounded net, hence
        /// bounded a fortiori (epic keystone soundness argument, architect
        /// GO design-review cycle-254). The field survives to the
        /// EventList projection ([`crate::passes::petri_to_events`]),
        /// which is the *codegen* contract; the runtime break is emitted
        /// there + in the backend (deferred to TASK-0341.02.01.05.04).
        ///
        /// Why a payload field and not a per-`IterVar` sidecar — same
        /// argument as `block_tag` above: the break predicate is a
        /// per-loop-**occurrence** fact, and `block_transform` reuses one
        /// `IterVar` across strip-mined passes. A per-`IterVar` map would
        /// conflate occurrences (the TASK-0180 double-count failure mode),
        /// and there is no stable per-loop-id substrate to key a sidecar
        /// on; the per-occurrence node is the only safe carrier. The field
        /// is also silent-sibling-safe: the compiler forces every
        /// construct site to set it (only the source-loop builder in
        /// [`crate::acfg::build_acfg`] ever sets `Some`).
        ///
        /// serde-default so an old wire payload (no field) deserialises
        /// as `None`; the carried `IrExpr` is the same node already in
        /// `ArgBinding::Scalar`, so the round-trip / determinism gate is
        /// already covered.
        #[cfg_attr(feature = "serde", serde(default))]
        break_cond: Option<IrExpr>,
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
/// [`build_acfg`](crate::acfg::build_acfg) notes).
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
    /// [`build_acfg`](crate::acfg::build_acfg) (i.e. before block-transform has run) and for
    /// programs whose schedule carries no `block=` directive.
    ///
    /// Why a sidecar set instead of a flag on [`ACFGNode::Repeat`]:
    /// keeping the variant payload stable means every existing
    /// pattern match on `Repeat { iter_var, range, body }` keeps
    /// compiling unchanged. The cost is a small lookup at the
    /// hoisting site, which is hot only for blocked schedules.
    #[cfg_attr(feature = "serde", serde(default))]
    pub inner_block_iter_vars: BTreeSet<IterVar>,

    /// Per-worker loop-range override for loops carrying a
    /// `partition=workers` schedule directive (TASK-0212). Populated by
    /// [`crate::passes::partition_workers`]. The outer key is the loop's
    /// [`IterVar`] (the one the source `for` declares); the inner map
    /// names each participating worker's exclusive iteration slice of
    /// the source range, so the union over workers re-covers the source
    /// range exactly once (B/N exact-divisible first cut). Loops with
    /// no `partition=workers` directive have no entry, and
    /// [`crate::passes::petri_to_events`] then projects the loop with
    /// the source range verbatim — unchanged from pre-TASK-0212.
    ///
    /// Why a sidecar map instead of a per-`Repeat` field: same reason
    /// as `inner_block_iter_vars` — keeps the
    /// `ACFGNode::Repeat { iter_var, range, body, block_tag }` payload
    /// stable so existing pattern matches keep compiling, and the
    /// per-worker projection in `petri_to_events::walk` already iterates
    /// workers individually for the `Event::Loop` emit, so the
    /// override is a small per-worker lookup at that one site.
    ///
    /// Determinism: outer `BTreeMap<IterVar, _>` keyed by id (a `u64`),
    /// inner `BTreeMap<WorkerId, Range<i64>>` keyed by id (a `u64`).
    /// Both iterate in numeric order; no `HashMap`/`HashSet`.
    #[cfg_attr(feature = "serde", serde(default))]
    pub partition_worker_ranges: BTreeMap<IterVar, BTreeMap<WorkerId, std::ops::Range<i64>>>,

    /// Pipeline-depth annotation for buffer places (TASK-0134, PRD §8.2
    /// "Initial marking on a place = pipeline depth / latency-hiding
    /// head-start"). Populated by [`crate::passes::transfer_inject`]
    /// when a Push/Wait pair is created inside a loop whose schedule
    /// carries `loop VAR : pipeline=D`. Key = the pair's
    /// [`SeqTag`]; value = the depth `D` (always `>= 2`; the schedule
    /// lowering rejects `pipeline=0` and `pipeline=1`).
    ///
    /// [`crate::passes::acfg_to_petri`] reads this sidecar at buffer-
    /// place creation time and sets `Place::initial_marking = D` for
    /// any seq present here; absence means `initial_marking = 0`
    /// (the default — every previously-translated example continues
    /// to start its buffer places empty).
    ///
    /// ## Semantics
    ///
    /// Interpretation (a) of PRD §8.2 (TASK-0134 design choice): every
    /// transfer in a pipelined loop body gets `initial_marking = D`.
    /// The producer is pre-credited with `D` head-start firings; the
    /// `buffer=N` capacity (>= D, enforced upstream by the link step)
    /// caps the in-flight count. Interpretation (b) — stage-decremented
    /// markings — was rejected for TASK-0134 because the ACFG carries
    /// no stage-numbering metadata, and the boundedness pass still
    /// polices (a) for soundness. See `acfg_to_petri.rs` "Initial
    /// markings" section.
    ///
    /// ## Innermost wins
    ///
    /// If a Push/Wait pair sits inside more than one pipelined loop
    /// (nested `pipeline=D1` ... `pipeline=D2` ...), the **innermost**
    /// enclosing pipelined loop's depth applies. The outer loop's
    /// depth is captured on a different (outer) buffer-place id by
    /// definition only if a separate Push/Wait pair was created at
    /// the outer scope. No example exercises nested pipelines today;
    /// the rule is fixed conservatively so the behaviour is defined.
    ///
    /// ## Determinism
    ///
    /// `BTreeMap<SeqTag, NonZeroU64>` — `SeqTag` is a `u64` newtype,
    /// `BTreeMap` iterates in numeric order. No `HashMap`/`HashSet` on
    /// any path that affects emitted output.
    #[cfg_attr(feature = "serde", serde(default))]
    pub pipeline_depth_for_seq: BTreeMap<SeqTag, std::num::NonZeroU64>,

    /// Per-(KernelId, IterVar) halo width inferred from the kernel's
    /// access pattern (TASK-0260, Stage 1). Populated by
    /// [`crate::passes::halo_inference::apply_halo_inference`]. The width
    /// `N` at `halo_widths[kid][iv]` is the maximum absolute integer
    /// offset across all of that kernel's `iter_var + b` DataRef indices
    /// along that axis; a kernel that reads `grid[y-1]`, `grid[y]`,
    /// `grid[y+1]` produces entry `halo_widths[blur3][y] = 1`.
    ///
    /// Independent of partition policy: halo widths apply whether the
    /// loop is partitioned or not. Stage 2 (TASK-0263, transfer_inject
    /// extension) consumes this map to extend per-tile transfer ranges;
    /// Stage 3 (TASK-0264, block-pair metadata recovery) couples it to
    /// `partition=blocks2d` neighbour resolution. The MAP itself is
    /// partition-agnostic — it is a property of the algorithm IR.
    ///
    /// Empty for algorithms whose kernels do not exercise affine
    /// `iter_var + b` reads (every example pre-Stage 2 ships this
    /// way, since Stage 1 records facts but no downstream pass yet
    /// observes them — Stage 1 lands the inference deliberately
    /// inert).
    ///
    /// ### Shape rationale: nested map, not `BTreeMap<(KernelId, IterVar), u64>`
    ///
    /// A tuple-keyed `BTreeMap<(KernelId, IterVar), u64>` would model
    /// the same fact more compactly but does not survive serde JSON
    /// (tuples cannot be JSON map keys). The nested `BTreeMap<KernelId,
    /// BTreeMap<IterVar, u64>>` shape mirrors
    /// [`Self::partition_worker_ranges`] — both keys are
    /// `serde(transparent)` `u64` newtypes that serialise as numeric
    /// JSON keys. The nested form is the shape that round-trips through
    /// the codegen-contract serde wire form (TASK-0233 precedent).
    ///
    /// Determinism: nested `BTreeMap` (`KernelId`, then `IterVar`),
    /// both `u64` newtypes; iteration is in numeric order. No
    /// `HashMap` / `HashSet` on the emit path. serde-default so an old
    /// wire payload (no field) deserialises as empty.
    #[cfg_attr(feature = "serde", serde(default))]
    pub halo_widths: BTreeMap<KernelId, BTreeMap<IterVar, u64>>,

    /// Per-(IterVar, DataId, axis) delay-line slot inferred from the
    /// loop body's affine-stride DataRef accesses when the schedule
    /// carries `loop V : reuse;` (TASK-0261, Stage 1). Populated by
    /// [`crate::passes::reuse_inference::apply_reuse_inference`]. The
    /// slot value names a circular buffer of `length` slots indexed by
    /// an offset in `[min_offset .. min_offset + length)` from the
    /// current iv value; the backend (Stage 2 / TASK-0265) consumes
    /// this to rewrite `grid[iv + b]` reads as
    /// `buf[(iv + b - min_offset) % length]` and load each row of the
    /// underlying array exactly once.
    ///
    /// Empty for algorithms whose schedule carries no `reuse`
    /// directives OR whose `reuse` loop body's iv-bearing offsets are
    /// length-1 (degenerate, dropped silently). serde-default so an
    /// older wire payload (no field) deserialises as empty — same
    /// additive contract as `halo_widths` (TASK-0260).
    ///
    /// ### Shape rationale: triple-nested map
    ///
    /// `BTreeMap<IterVar, BTreeMap<DataId, BTreeMap<u64 /* axis */,
    /// ReuseSlot>>>` — three levels of `BTreeMap` keyed by `u64`-like
    /// newtypes / a plain `u64` axis index. The deep-nest shape is
    /// load-bearing for serde-JSON: a flat tuple-keyed `BTreeMap<(IterVar,
    /// DataId, u64), ReuseSlot>` would not round-trip (tuples cannot
    /// be JSON map keys). The nested form mirrors
    /// [`Self::halo_widths`] and `Self::partition_worker_ranges` (both
    /// nested for the same reason).
    ///
    /// Independent of partition policy: a delay line lives inside one
    /// worker's tile; partitioning bounds the iv-range each worker
    /// covers but the per-iteration reuse rewrite is unaffected.
    ///
    /// Determinism: nested `BTreeMap`s; iteration in numeric order at
    /// every level. No `HashMap` / `HashSet` on the emit path.
    #[cfg_attr(feature = "serde", serde(default))]
    pub reuse_widths: BTreeMap<
        IterVar,
        BTreeMap<DataId, BTreeMap<u64, crate::passes::reuse_inference::ReuseSlot>>,
    >,

    /// Per-outer-`IterVar` pairing of the two iter-vars a single
    /// `partition=blocks2d` directive partitions (TASK-0264 cycle 113,
    /// AC#1). Populated by
    /// [`crate::passes::partition_blocks2d::apply_partition_blocks2d`]
    /// alongside its [`Self::partition_worker_ranges`] writes.
    /// Outer key = the outer (row-band) [`IterVar`]; value = the
    /// paired inner (col-band) [`IterVar`] for the same blocks2d
    /// directive.
    ///
    /// ## Why a sidecar — not re-derivation
    ///
    /// `partition_worker_ranges` records TWO entries per blocks2d
    /// directive (one per axis), with the same `WorkerId` keyset and
    /// the per-worker (row, col) assignment implied by BTreeSet
    /// iteration order. A downstream pass (TASK-0289 halo-strip
    /// Push/Wait synthesis) cannot distinguish "paired-by-one-
    /// blocks2d-directive" from "two independent partition=rows
    /// directives on unrelated loops" by reading
    /// `partition_worker_ranges` alone. This sidecar captures the
    /// pairing at the source — at the partition pass — so the
    /// consumer reads it instead of re-deriving by walking
    /// `linked.sched.loops`.
    ///
    /// ## Cross-reference: [`Self::grid_shape_for_outer_iv`]
    ///
    /// The pair + the grid shape together let a downstream pass
    /// invert WorkerId → (row, col) without re-running
    /// `decompose_grid`. Both fields are populated in lockstep by
    /// `partition_blocks2d`.
    ///
    /// ## Determinism
    ///
    /// `BTreeMap` keyed by `IterVar` (a `u64` newtype); iteration in
    /// numeric order. serde-default so an old wire payload (no field)
    /// deserialises as empty (no partition_blocks2d directives —
    /// equivalent to the pre-TASK-0264 codegen behaviour).
    #[cfg_attr(feature = "serde", serde(default))]
    pub partition_pairs: BTreeMap<IterVar, IterVar>,

    /// Per-outer-`IterVar` `(rows, cols)` grid shape inferred by
    /// `partition_blocks2d`'s `decompose_grid(num_workers)` call
    /// (TASK-0264 cycle 113, AC#2). Populated by
    /// [`crate::passes::partition_blocks2d::apply_partition_blocks2d`]
    /// alongside its [`Self::partition_worker_ranges`] +
    /// [`Self::partition_pairs`] writes. Key = the outer
    /// [`IterVar`] (the same key used in `partition_pairs` and in
    /// `partition_worker_ranges`); value = `(rows, cols)`.
    ///
    /// ## Why a sidecar — not exposing `decompose_grid`
    ///
    /// `partition_blocks2d::decompose_grid` is the
    /// canonical factoriser today (largest-square-or-closest-to-
    /// square deterministic decomposition; rejects prime-worker
    /// counts as degenerate). Exposing it as `pub(crate)` would
    /// force every downstream consumer to re-invoke it with the same
    /// worker count, and to KNOW the worker count for that outer iv
    /// — information already known at the partition pass site.
    /// Persisting the grid shape in the sidecar makes the consumer
    /// lookup O(log n) and eliminates the re-derivation risk if
    /// `decompose_grid`'s tiebreaker policy changes (the sidecar is
    /// the single source of truth for which grid shape was actually
    /// used).
    ///
    /// ## Worker → (row, col) inversion
    ///
    /// A downstream pass (TASK-0289) inverts a WorkerId to its
    /// (row, col) cell via `i = bset_position(worker)` then
    /// `(row, col) = (i / cols, i % cols)` where `(rows, cols) =
    /// grid_shape_for_outer_iv[outer_iv]`. The body_workers ordering
    /// is `BTreeSet::iter()` (numeric), matching `partition_blocks2d`'s
    /// row-major assignment.
    ///
    /// ## Determinism
    ///
    /// `BTreeMap` keyed by `IterVar` (a `u64` newtype); iteration in
    /// numeric order. serde-default so an old wire payload (no field)
    /// deserialises as empty.
    #[cfg_attr(feature = "serde", serde(default))]
    pub grid_shape_for_outer_iv: BTreeMap<IterVar, (u32, u32)>,
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
