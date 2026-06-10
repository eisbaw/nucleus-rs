//! Event types — the presentation-layer contract from PRD §8.3.
//!
//! The schedule pass projects the `GlobalNet` (PRD §8.2) onto each
//! worker as an ordered `EventList`. The backends in §7 consume this
//! `EventList` directly: every backend lowers the same six events to
//! its own primitives (threads, MPI calls, DMA descriptors, …). This
//! module defines the event vocabulary and the supporting types.
//!
//! See PRD §8.3 for the authoritative description. TASK-0015 is the
//! task that introduced this module.
//!
//! Why a separate module (sibling to `algo`, `sched`, `link`) and not
//! a submodule of `sched`: the events are the *output* of the schedule
//! pass, not part of its input. Co-locating with the SchedIR would
//! confuse the boundary. Putting it next to `link` matches the
//! pipeline direction: `algo`/`sched` -> `link` -> (later) `event`.
//!
//! ## Design choices recorded for posterity
//!
//! - **Opaque `u64` identifier newtypes** for `KernelId`, `DataId`,
//!   `WorkerId`, `IterVar`, `SeqTag`, and `Region`. The compiler
//!   assigns these as it lowers; the human-readable name (e.g.
//!   `"conv_block_1"`, `"y"`, `"shared_sram"`) is carried by a
//!   separate sidecar map (out of scope here). Rationale:
//!   - Cheap equality and hashing (no string interning needed at this
//!     layer).
//!   - Backends never need to parse identifiers; they look them up.
//!   - Easy to round-trip through serde without lifetime gymnastics.
//!
//!   Trade-off: errors phrased in terms of raw IDs are not
//!   human-readable until the sidecar is consulted. Inspection
//!   tooling must thread the name map. Acceptable for an internal
//!   contract; revisit if examples demand otherwise.
//!
//! - **`Region` is an opaque `u64`**, not a string. The PRD §8.3
//!   says the backend interprets it; the compiler treats it as an
//!   ID assigned during scheduling. Matches the schedule's
//!   `place_data D in MEMORY_REGION` lowering: the schedule resolves
//!   `MEMORY_REGION` to an integer index into the backend's region
//!   table.
//!
//! - **`SyncKind` is an `enum` with a single `Barrier` variant**, not
//!   a unit struct. PRD §8.3 explicitly forecasts other variants
//!   (`Rendezvous`, `Quorum`) under specific evidence; keeping the
//!   `enum` shape means adding a variant later does not break the
//!   public API. Costs one `match` arm in callers today; pays for
//!   itself the first time a second variant lands.
//!
//! - **`IterTile` is a `Vec<(IterVar, Range<i64>)>`** in iteration-
//!   nest order, wrapped in a named struct (not a bare type alias).
//!   Reasons:
//!   - The struct gives us a place to hang methods (`is_empty`,
//!     `rank`) and document the ordering invariant.
//!   - The named type makes downstream signatures self-documenting.
//!   - The element type is `(IterVar, Range<i64>)`, not a custom
//!     pair, so pattern matching stays cheap.
//!
//!   `Range<i64>` not `RangeInclusive`: PRD §8.3 specifies
//!   half-open. `i64` not `u64`: schedule directives in the PRD
//!   examples use plain integer arithmetic that can produce negative
//!   intermediate values (e.g. halo offsets). Concrete schedule
//!   lowering can normalise later if it wants unsigned bounds; the
//!   event type stays permissive.
//!
//!   Equality of `IterTile`: two tiles with the same `(var, range)`
//!   sequence are equal. `Range` implements `PartialEq` and `Eq`,
//!   `Vec` does too, so the derive is straight. We do NOT canonicalise
//!   on construction (no sorting by `IterVar`): the iteration-nest
//!   order *is* semantically meaningful (it tells the backend which
//!   axis is the outer loop), so reordering would lose information.
//!
//! - **`Sync` uses `BTreeSet<WorkerId>`** for `participants`. The PRD
//!   types it as `Set<WorkerId>`; concretely we want a `Set` with
//!   stable iteration order, for deterministic codegen and run-stable
//!   hashing/serialisation. `HashSet`'s nondeterministic iteration
//!   fails that; `BTreeSet<WorkerId>` satisfies it. (`Event`'s `Hash`
//!   is hand-written since TASK-0159 — see "Why `Hash` is
//!   hand-written" below; the participant set must still be
//!   order-stable so that hand-written `Hash` and the serialised form
//!   are identical across runs.)
//!
//! - **`Sync` carries a stable cross-worker `sync: SyncTag`**
//!   (TASK-0172) — the `Sync` analogue of `seq` on `Push`/`Wait`.
//!   Every participant of one barrier records the same `SyncTag`, so
//!   disjoint per-worker `EventList`s identify the barrier without a
//!   global ACFG walk. This replaced the backend pre-order-Sync-index
//!   heuristic that was correct only for *uniform* barriers; with the
//!   carried tag, partial / non-uniform barriers lower correctly.
//!   The tag is included in the hand-written `Hash` (it is part of
//!   event identity for dedup/golden paths).
//!
//! - **Serde support is gated behind the default-on `serde` feature.**
//!   The contract is the inter-stage wire format (schedule pass ->
//!   backend codegen, plus golden-test fixtures). Default-on so the
//!   common path needs no opt-in; opt-out exists for size-constrained
//!   builds that don't need serialisation. The feature also pulls in
//!   serde for `BTreeSet`/`Range`, which both ship serde impls
//!   natively when the `serde` feature is enabled on the `serde`
//!   crate (which we do).
//!
//! ## What this module deliberately does NOT do
//!
//! - **No span / source-location info on events.** Diagnostics that
//!   need to point at the algorithm or schedule line that produced an
//!   event will need a sidecar map; events themselves stay lean.
//!   Filed as a follow-up.
//! - **No `Event::Latency`-style measurement events.** PRD §6.3.5
//!   talks about `check` directives' measurement points, but the
//!   measurement model is TBD. Filed as a follow-up.
//! - **No validation here** of e.g. `Push.dst != self`, matched
//!   `Push`/`Wait` `seq` pairing, non-empty `Sync.participants`.
//!   That belongs to the scheduler/validator that *constructs*
//!   events, not the type module.

use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};
use std::ops::Range;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::algo::IrExpr;

// --------------------------------------------------------------------
// Opaque identifier newtypes
// --------------------------------------------------------------------

/// Identifier for a kernel as declared in the algorithm. Assigned by
/// the compiler during lowering; the original textual name lives in a
/// sidecar map (see module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct KernelId(pub u64);

/// Identifier for a data symbol declared in the algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct DataId(pub u64);

/// Identifier for a worker in the schedule's worker table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct WorkerId(pub u64);

/// Identifier for an iteration variable (e.g. the `y` in `for y : ..`).
/// Scoped to the algorithm; assigned by the lowering pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct IterVar(pub u64);

/// Compile-time sequence number on a `Push`/`Wait` pair. Two events
/// match if and only if they carry the same `(src, dst, data, tile,
/// seq)`.
///
/// **Implementation invariant (load-bearing for TASK-0233)**: assigned
/// from a SINGLE GLOBAL MONOTONIC COUNTER by
/// `passes::transfer_inject` (the `next_seq` field on its `State`,
/// handed out by `State::fresh_seq` in
/// `passes/transfer_inject/mod.rs`), NOT per-(src, dst, data) triple.
/// So every SeqTag in a single program is
/// unique globally — different transfers (different DataIds) never
/// share a SeqTag. This stronger guarantee lets
/// `NameSidecar::xfer_facts` use `SeqTag` alone as the
/// key (rather than `(DataId, SeqTag)`); a future cycle that
/// shards the counter per-triple MUST also widen the sidecar key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct SeqTag(pub u64);

/// Stable cross-worker barrier identity on a [`Event::Sync`] — the
/// `Sync` analogue of [`SeqTag`] on `Push`/`Wait` (TASK-0172).
///
/// Every participant of one barrier records an `Event::Sync` carrying
/// the **same** `SyncTag`; two `Event::Sync`s carry the same tag if
/// and only if they are the same barrier. This is the cross-worker
/// join key that lets disjoint per-worker `EventList`s agree on
/// barrier identity *without* a global ACFG walk — a partial /
/// non-uniform barrier (participant sets that differ between barriers)
/// is then lowered correctly because identity no longer relies on
/// every participant seeing the same prefix of barriers.
///
/// A distinct newtype (not a reused [`SeqTag`]) because barriers and
/// data transfers are different identity domains: sharing the space
/// would let a barrier alias a transfer in any `seq`-keyed code path
/// and erases a type-level distinction the compiler should keep.
///
/// Assigned by the sync-injection pass (the analogue of where
/// [`crate::acfg::XferPlaceholder::seq`] is assigned — the site where
/// the *global* barrier structure is visible), monotonically in a
/// deterministic pre-order walk, then threaded verbatim through
/// `petri_to_events` into `Event::Sync`.
// `Default` (⇒ `SyncTag(0)`) is derived only so `SyncPlaceholder`
// (which derives `Default` for test/builder ergonomics) keeps that
// derive. The default value is never semantically meaningful: every
// real `SyncPlaceholder` gets its tag overwritten by
// `sync_inject::assign_sync_tags`. (`SeqTag` is *not* `Default`
// because `XferPlaceholder` does not derive `Default`.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct SyncTag(pub u64);

/// Opaque backend-interpreted memory region handle. The compiler
/// assigns this index when lowering `place_data D in MEMORY_REGION`;
/// the backend's `capabilities.toml` decides what physical memory each
/// index corresponds to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Region(pub u64);

// --------------------------------------------------------------------
// IterTile
// --------------------------------------------------------------------

/// Rectangular slice of iteration space. One half-open `i64` interval
/// per iteration variable, in iteration-nest order (outer-most first).
///
/// - For a `Fire`, the tile names the iteration coordinates this
///   firing covers.
/// - For `Alloc` / `Push` / `Wait` / `Free`, the tile names the slice
///   of the named `data` symbol involved — derived from the kernel's
///   declared access pattern projected onto the firing's tile.
/// - For non-iterated firings (top-level dataflow statements), the
///   tile is empty (`bounds.is_empty()`).
///
/// Equality is by full `(IterVar, Range)` sequence in order; two
/// tiles with the same bounds and same iter-var order compare equal.
/// The order is significant and not normalised on construction.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct IterTile {
    /// Outer-most iteration variable first. Empty for non-iterated
    /// firings.
    ///
    /// **Convention is load-bearing:** downstream passes rely on the
    /// outer-to-inner ordering. In particular, `transfer_inject::
    /// annotate_pipeline_depth_for_seq` (TASK-0134) walks
    /// `bounds.iter().rev()` to find the innermost pipelined iter-var.
    /// All canonical construction sites build outer-to-inner naturally:
    /// the enclosing-tile stack in `transfer_inject` Push/Wait
    /// creation, the fan-out path, and (since TASK-0224) the
    /// partition-rewrite path, which iterates partitioned iter-vars in
    /// program-wide nest order derived from a DFS pre-order over the
    /// ACFG's `Repeat` nodes rather than relying on the `IterVar` id
    /// ascending = nest order coincidence the earlier
    /// `BTreeMap<IterVar, ...>` key iteration depended on.
    pub bounds: Vec<(IterVar, Range<i64>)>,
}

impl IterTile {
    /// The empty tile (no iteration). Use for top-level dataflow
    /// statements that are not inside any `for`.
    pub fn empty() -> Self {
        IterTile { bounds: Vec::new() }
    }

    /// Construct from the `(var, range)` sequence in nest order.
    pub fn new(bounds: Vec<(IterVar, Range<i64>)>) -> Self {
        IterTile { bounds }
    }

    /// True iff the tile names zero iteration variables.
    pub fn is_empty(&self) -> bool {
        self.bounds.is_empty()
    }

    /// Number of iteration variables this tile ranges over (the rank
    /// of the iteration sub-space, NOT the volume of points).
    pub fn rank(&self) -> usize {
        self.bounds.len()
    }
}

// Manual Hash for IterTile.
//
// `std::ops::Range<i64>` does not implement `Hash` (it implements
// `IntoIterator`, and there's a long-standing decision to keep `Hash`
// off it). We hash field-by-field instead so callers can still put
// `Event` and `IterTile` into hash-based containers.
impl std::hash::Hash for IterTile {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Length first so a prefix can't collide with a different-length
        // tile that hashes the same elements.
        self.bounds.len().hash(state);
        for (v, r) in &self.bounds {
            v.hash(state);
            r.start.hash(state);
            r.end.hash(state);
        }
    }
}

// --------------------------------------------------------------------
// SyncKind
// --------------------------------------------------------------------

/// Control-only synchronisation flavour. PRD §8.3 ships exactly one
/// variant. Kept as an `enum` (not a unit struct) so future variants
/// (`Rendezvous`, `Quorum`) can be added without breaking the public
/// API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum SyncKind {
    /// All listed participants arrive; then all proceed.
    Barrier,
}

// --------------------------------------------------------------------
// Per-Fire value bindings (TASK-0156)
// --------------------------------------------------------------------

/// One indexed access to a data symbol — `D[idx0][idx1]…` (or bare
/// `D` for a scalar / whole-array reference, `indices` empty).
///
/// `indices` carries the AlgoIR [`IrExpr`] index expressions
/// verbatim, outer dimension first (e.g. `img_in[y-1][x+1]` ⇒
/// `[y-1, x+1]`). We reuse the AlgoIR expression type rather than
/// inventing a third index grammar: it is the single source of truth
/// for what an index *is*, it is inert data at this layer (no pass
/// evaluates it here), and TASK-0150 already gave it serde derives.
///
/// This is the same shape as `acfg::DataAccess`; `acfg` re-exports
/// *this* type (see `acfg.rs`) so the ACFG and the Event contract
/// share one definition instead of two that must be kept in lockstep.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct DataSlice {
    /// The data symbol read or written.
    pub data: DataId,
    /// Per-axis index expressions, outer dimension first. Empty for a
    /// scalar or whole-array (un-indexed) reference.
    pub indices: Vec<IrExpr>,
}

/// One positional kernel argument of a [`Event::Fire`].
///
/// PRD §6.2.3: a kernel argument is one of
///
/// - an indexed (or whole-array) read of a data symbol — `a[i]`,
///   `img_in[y-1][x]`, bare aggregate `img_out` ⇒ [`ArgBinding::Data`];
/// - a scalar arithmetic expression over iteration vars / consts (no
///   data reference inside) ⇒ [`ArgBinding::Scalar`];
/// - a **nested kernel call** whose own arguments recurse the same
///   shape — `denoise(mix2(mic_in[frame], bt_in[frame]))`
///   (example 14) ⇒ [`ArgBinding::Nested`].
///
/// The nested-call form is faithfully represented here even though
/// the tier-1 backends (the shared
/// `backend_common::render::fire::render_fire_arg` helper) currently
/// *reject* it: the EventList contract must mirror what the program
/// *is*, and the decision "this backend can't lower a nested call in
/// argument position" belongs to the backend, not to ACFG/Event
/// construction. Earlier passes (`build_acfg`) previously accepted
/// example 14 into an ACFG (its `data_in` recursed into the nested
/// call); the binding must not regress that by panicking. Whether a
/// future backend lowers nested calls is its own concern.
///
/// The variant order mirrors how a backend lowers the argument:
/// `Data` ⇒ index into the symbol's backing store; `Scalar` ⇒ emit
/// the integer expression directly; `Nested` ⇒ (if supported) emit
/// the inner call, recursively.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum ArgBinding {
    /// An indexed (or whole-array) read of a data symbol.
    Data(DataSlice),
    /// A scalar arithmetic expression over iteration vars / consts
    /// (no data reference inside). Carried verbatim as an
    /// [`IrExpr`].
    Scalar(IrExpr),
    /// A nested kernel call in argument position. `callee` is the
    /// textual kernel name (same representation as `IrExpr::Call`);
    /// `args` are its arguments, bound by the same rules,
    /// recursively.
    Nested {
        callee: String,
        args: Vec<ArgBinding>,
    },
}

/// The per-firing value binding attached to a [`Event::Fire`]: which
/// `(DataId, slice)` / scalar expression feeds each kernel parameter,
/// in parameter order, and which `(DataId, slice)` the firing writes.
///
/// This is the payload that lets a backend compute the *value* of a
/// firing from the `EventList` alone (TASK-0156 / unblocks
/// TASK-0124). Before this, `Event::Fire` carried only `kernel` +
/// `tile`, so a backend had to walk the AlgoIR to know what to feed
/// the kernel and where to store the result.
///
/// `output` is `None` for effect statements (kernel returns `()`),
/// `Some` for dataflow statements (`d <-- k(...)`). For a top-level
/// non-iterated firing the slices' index lists are whatever the
/// source wrote (often empty / whole-array).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct FireBinding {
    /// One entry per kernel parameter, in declared parameter order.
    pub inputs: Vec<ArgBinding>,
    /// The `(DataId, slice)` the firing writes, or `None` for an
    /// effect firing.
    pub output: Option<DataSlice>,
}

/// Per-occurrence strip-mine rebinding tag (TASK-0180).
///
/// ## Why this exists — per-OCCURRENCE, not per-IterVar
///
/// `block_transform` strip-mines `for VAR : LO..HI block=N` into a
/// `(tile-loop, inner-loop)` nest and **reuses `VAR`'s [`IterVar`]**
/// on the inner loop. Codegen must therefore expand the inner loop
/// variable to its *absolute* source value at every body use site
/// (`LO + tile*N + inner` for a full/divisible nest;
/// `LO + num_full*N + inner` for the trailing partial tile).
///
/// The pre-TASK-0180 backend re-derived "is this a rebindable inner
/// loop" from a program-**global** `Event::Loop` occurrence count
/// (`divisible_inner_block_vars`, `counts==1`). That conflates three
/// cases that share one reused `IterVar`:
///
/// 1. a divisible single-nest (`count==1` — correctly rebound);
/// 2. a non-divisible full+partial sibling pair (`count==2`, same
///    var, different ranges — TASK-0173);
/// 3. a loop-var **name** legitimately reused across N independent
///    evenly-divisible passes (`count==N` — *wrongly excluded*, so an
///    accumulator runs `tiles*range` instead of `range` and is
///    exactly N×/2× wrong; this is the 04-prefix-sum/blocked bug).
///
/// A per-`IterVar` sidecar map (or the `inner_block_iter_vars`
/// `BTreeSet<IterVar>`) **cannot** distinguish these — they all key on
/// the *same reused id*. The distinction is per-loop-**occurrence**, so
/// (mirroring the [`FireBinding`] / TASK-0156 precedent: per-event
/// facts go ON the event, per-program facts in the sidecar) the tag
/// is an additive field on the [`Event::Loop`] node itself, originated
/// by `block_transform` (the only site that knows `N`/`num_full`/the
/// full-vs-partial split). The backend rebinds **purely** from this
/// per-occurrence tag — no global count, no heuristic.
///
/// `LO` (the source lower bound) is *not* carried here: it is the
/// same for every reused occurrence and already lives in
/// `NameSidecar::loop_bounds` keyed by the (reused) `IterVar` —
/// re-deriving it here would duplicate a single source of truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct BlockTag {
    /// The block width `N` (`block=N`). For a full/divisible nest the
    /// inner loop's trip count *is* `N`; for the trailing partial tile
    /// it is the remainder (`< N`) but the absolute-index stride is
    /// still `N`.
    pub block_n: i64,
    /// Number of whole tiles of width `N` that precede the trailing
    /// partial tile (`(HI-LO) / N`). For the partial tile this is the
    /// constant tile-offset (`LO + num_full*N + inner`); for the full
    /// nest it equals the tile loop's iteration count and is unused in
    /// the rebinding formula (the tile var supplies the offset).
    pub num_full: i64,
    /// `false` for the full/divisible nest (rebind
    /// `LO + tile*N + inner`, `tile` = enclosing tile-loop var);
    /// `true` for the trailing partial tile (rebind
    /// `LO + num_full*N + inner` — its own tile loop is `0..1` so
    /// `tile*N` would be `0`, the wrong base).
    pub is_partial: bool,
}

// --------------------------------------------------------------------
// CheckFrame  (TASK-0052.02)
// --------------------------------------------------------------------

/// On-violation action for a real-time `check loop V : latency_max=T`
/// directive (PRD §6.3.5).
///
/// This is the **codegen-layer** carrier: the same three variants as
/// `sched::ast::ViolationKind`, intentionally duplicated rather than
/// re-exported so the `event` module stays independent of `sched`
/// (the EventList is the *output* of scheduling and downstream-only;
/// adding a back-edge `event -> sched` would invert the dep graph and
/// break the `algo / sched / link -> event` pipeline direction stated
/// in this module's docstring).
///
/// The bridge from `sched::ast::ViolationKind` to this lives in the
/// check-frame projection pass (`passes::inject_check_frames`); the
/// two enums have one-to-one structure so the conversion is total.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum ViolationKind {
    /// Default per PRD §6.3.5: abort the worker process with a
    /// distinct exit signature naming the loop_var + measured ns +
    /// threshold ns. The cross-backend differential distinguishes
    /// "panic-on-violation" (exit code 101 + empty stdout) from
    /// "wrong output" cleanly.
    Panic,
    /// `eprintln!` once per violation; execution continues. The
    /// generated stdout is unchanged on violation, so determinism on
    /// the output channel is preserved (the message goes to stderr).
    /// Wired at tier-1 (`backend-common::check_frame`, TASK-0052.04)
    /// and tier-3 embedded (TASK-0048.04, per-violation UART line).
    Log,
    /// Increment an atomic counter; print a one-line summary to
    /// stderr at run end (`Drop` on a guard struct). Determinism on
    /// stdout is preserved (the count goes to stderr).
    /// Wired at tier-1 (`backend-common::check_frame` AtomicU64 + Drop
    /// summary, TASK-0052.04) and tier-3 embedded (TASK-0048.08,
    /// AtomicU32 + program-exit USART1 summary; AtomicU64 is absent on
    /// thumbv7em and a spinning firmware never fires a `Drop`).
    Count,
}

/// Per-`Event::Loop` real-time assertion frame (TASK-0052.02).
///
/// ## Why an additive field on `Event::Loop` (not a new event variant)
///
/// The loop boundary is already a first-class concept here — `Event::
/// Loop` mirrors `ACFGNode::Repeat` one-for-one. A `CheckBegin/End`
/// pair bracketing the loop would (a) project ambiguously onto
/// per-worker EventLists (which side of a partial-participation
/// barrier does the bracket sit on?), (b) duplicate the
/// loop-identity join key (`iter_var`) into a separate event, and
/// (c) require every existing consumer (boundedness, deadlock,
/// reconstruction tests, backends) to acquire bracket-pairing logic.
/// An optional annotation, mirroring the `block_tag` precedent on the
/// SAME variant, is the minimal change.
///
/// ## Source of the field — projection-time join, not lowering-time
///
/// `sched_ir.checks: BTreeMap<String, ResolvedCheckDirective>` is
/// keyed by loop-variable NAME; `Event::Loop.iter_var` is an opaque
/// `IterVar` id. The `acfg.name_iter_vars: BTreeMap<String, IterVar>`
/// map is the join key. The projection pass
/// `passes::inject_check_frames` performs that join AFTER
/// `acfg_to_events`, so the `acfg_to_events` signature stays
/// `acfg -> per_worker` (every existing test call site is unchanged).
///
/// ## Which loops carry a frame
///
/// Only the OUTER user-source loop matches a `check loop V` directive
/// (the user writes the source-loop name, not a strip-mined tile name).
/// `block_tag.is_some()` ⇒ this loop is a strip-mine-synthesised inner
/// loop; the projection pass skips it. A user who writes
/// `check loop tile : ...` on a tile-loop name (synthesised by
/// `block_transform`) does not match — the projection pass leaves
/// `check_frame = None`, since the strip-mining is implementation
/// detail rather than a source-visible loop.
///
/// ## Default-materialisation rationale
///
/// `on_violation` materialises `ViolationKind::Panic` here (the
/// projection seam) when the user's `check loop V` directive omits
/// `on_violation = ...`. The IR (`sched::ir::ResolvedCheckDirective`)
/// stays faithful to what the user wrote (zero `OnViolation` entries
/// when the user wrote none — TASK-0052.01 forward-carry note); the
/// codegen-layer default is materialised here, NOT at IR.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct CheckFrame {
    /// `latency_max = T` normalised to nanoseconds (the source unit
    /// is preserved on `sched::ast::TimeLit` for diagnostics; once it
    /// reaches codegen the comparison uses nanoseconds exclusively).
    /// Guaranteed `> 0` — `latency_max = 0` is rejected at sched-lower
    /// (TASK-0052.01, `ZeroLatencyMax`).
    pub latency_max_ns: u64,
    /// Resolved on-violation action (codegen-layer default applied —
    /// see struct docstring).
    pub on_violation: ViolationKind,
    /// The source loop-variable name (`check loop V` -> `V`).
    /// Carried by VALUE so the backend produces a precise panic
    /// message without a back-reference to `NameTables`.
    ///
    /// **Duplication-vs-NameTables (TASK-0221):** backends ALSO have
    /// the iter-var name available via `NameTables.iter_var[iter_var]`
    /// where `iter_var` is the enclosing `Event::Loop`'s field. These
    /// two MUST name the same identifier. The defensive-assert path
    /// was chosen: each backend's Event::Loop arm includes a
    /// `debug_assert_eq!(var.as_str(), frame.loop_var.as_str(), ...)`
    /// before any emit that uses `frame.loop_var`. Dev builds catch
    /// projection-layer divergence loudly; release builds skip the
    /// check (zero cost on the codegen path). Alternative considered:
    /// drop the field and look up via NameTables at emit time —
    /// rejected because it would require threading NameTables
    /// through `collect_count_check_frames` (an EventList walker that
    /// has no NameTables today), expanding API surface for marginal
    /// architectural gain.
    pub loop_var: String,
}

impl FireBinding {
    /// The empty binding: no inputs, no output. Used by synthetic
    /// callers / tests that do not model values, and by the
    /// `Event::fire_bare` convenience.
    pub fn none() -> Self {
        FireBinding {
            inputs: Vec::new(),
            output: None,
        }
    }

    /// True iff this binding carries no value information at all.
    pub fn is_empty(&self) -> bool {
        self.inputs.is_empty() && self.output.is_none()
    }
}

// Manual Hash for the binding types. `IrExpr` deliberately does not
// implement `Hash` (it mirrors the AST, which doesn't), and `Event`
// must stay `Hash` (it goes into `HashSet` in tests / dedup paths).
// We hash the structural skeleton; collisions across structurally
// distinct expressions are acceptable for a `Hash` impl (equality is
// still exact via the derived `PartialEq`).
impl std::hash::Hash for DataSlice {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.data.hash(state);
        self.indices.len().hash(state);
    }
}

impl std::hash::Hash for ArgBinding {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            ArgBinding::Data(d) => {
                0u8.hash(state);
                d.hash(state);
            }
            ArgBinding::Scalar(_) => {
                1u8.hash(state);
            }
            ArgBinding::Nested { callee, args } => {
                2u8.hash(state);
                callee.hash(state);
                args.len().hash(state);
                for a in args {
                    a.hash(state);
                }
            }
        }
    }
}

impl std::hash::Hash for FireBinding {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.inputs.len().hash(state);
        for a in &self.inputs {
            a.hash(state);
        }
        self.output.hash(state);
    }
}

// --------------------------------------------------------------------
// Event
// --------------------------------------------------------------------

/// The six event variants per PRD §8.3. Each event is the projection
/// of a transition firing onto the worker that owns it. A worker's
/// `EventList` is a `Vec<Event>` in execution order (the ACFG layer
/// — TASK-0016, Done — owns the actual emission; this module just
/// defines the type).
///
/// Wire format (with `serde` feature on): externally tagged JSON by
/// default — e.g. `{"Fire": {"kernel": 0, "tile": {...}, "bindings":
/// {...}}}` and `{"Loop": {"iter_var": 0, "range": {"start": 0,
/// "end": 4}, "body": [ ... ]}}` (nested `Event`s; TASK-0159).
/// Backends and golden tests can rely on this shape.
///
/// ## Why `Hash` is hand-written (not derived) — TASK-0159
///
/// The `Loop` variant (added by TASK-0159) carries `range:
/// Range<i64>`, and `std::ops::Range` deliberately does *not*
/// implement `Hash` (same long-standing std decision that made
/// [`IterTile`] / [`FireBinding`] hand-roll their `Hash`). `Event`
/// must stay `Hash` (it goes into `HashSet` in tests / dedup paths),
/// so the whole enum gets a manual `Hash` that hashes the structural
/// skeleton and recurses through a `Loop` body. Equality stays exact
/// via the derived `PartialEq`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Event {
    /// Execute kernel `kernel` over iteration coordinates `tile`,
    /// with the per-firing value binding `bindings` (TASK-0156):
    /// the ordered `(DataId, slice)` / scalar inputs feeding each
    /// kernel parameter and the `(DataId, slice)` it writes. `tile`
    /// is empty for top-level (non-iterated) firings;
    /// `bindings.is_empty()` for synthetic firings that do not model
    /// values (see [`Event::fire_bare`]).
    Fire {
        kernel: KernelId,
        tile: IterTile,
        bindings: FireBinding,
    },

    /// Reserve backing storage for `data` over the iteration slice
    /// `tile` in memory region `region`. The backend interprets
    /// `region`; the compiler treats it as opaque.
    Alloc {
        data: DataId,
        tile: IterTile,
        region: Region,
    },

    /// Send the slice `(data, tile)` to worker `dst`. Pairs with a
    /// `Wait` on `dst` carrying the same `seq`.
    Push {
        dst: WorkerId,
        data: DataId,
        tile: IterTile,
        seq: SeqTag,
    },

    /// Receive the slice `(data, tile)` from worker `src`. Pairs with
    /// a `Push` on `src` carrying the same `seq`.
    Wait {
        src: WorkerId,
        data: DataId,
        tile: IterTile,
        seq: SeqTag,
    },

    /// Control-only synchronisation across `participants`. No data
    /// crosses; progress waits for every participant to arrive.
    ///
    /// `sync` is the stable cross-worker barrier identity (TASK-0172):
    /// every participant of one barrier carries the *same* [`SyncTag`],
    /// so disjoint per-worker `EventList`s agree on which barrier this
    /// is without a global ACFG walk. This is the `Sync` analogue of
    /// `seq` on `Push`/`Wait`; it makes partial / non-uniform barriers
    /// (participant sets that differ between barriers) lowerable, since
    /// barrier identity no longer depends on a per-worker pre-order
    /// index that only coincides for uniform barriers.
    Sync {
        participants: BTreeSet<WorkerId>,
        kind: SyncKind,
        sync: SyncTag,
    },

    /// Release backing storage for `(data, tile)`. The region was
    /// fixed by the matching `Alloc`; the backend looks it up.
    Free { data: DataId, tile: IterTile },

    /// A rolled loop: execute `body` once per value of `iter_var` in
    /// the half-open `range`, in order (TASK-0159).
    ///
    /// ## Why this exists — structure-preserving projection
    ///
    /// The EventList projection used to *unroll* every
    /// [`crate::acfg::ACFGNode::Repeat`] into `range.len()` flat copies
    /// of the enclosed events (matching the analysis Net, which still
    /// unrolls — see `acfg_to_petri`). A backend that consumes ONLY the
    /// EventList (TASK-0124) then cannot tell "a `for y in 1..15` over
    /// one Fire" from "15 unrelated Fires": the loop variable name, the
    /// bound, and the for-structure are gone. It cannot re-emit a
    /// rolled loop, so it cannot be byte-identical to the AlgoIR-walking
    /// backend. `Event::Loop` is the loop-structure analogue of what
    /// TASK-0156 did for value bindings: the projection now carries the
    /// loop nest verbatim so the contract is self-sufficient.
    ///
    /// `body: Vec<Event>` is the per-worker projection of the
    /// `Repeat`'s body sub-tree, recursively (a nested `Repeat`
    /// projects to a nested `Loop`). It mirrors
    /// [`crate::acfg::ACFGNode::Repeat`] one-for-one; the structure is
    /// not flattened.
    ///
    /// ## Trailing partial tile (forward-carried from TASK-0142)
    ///
    /// `block_transform`/tiling decomposes a strip-mined loop into a
    /// `Sequence[ full-tile nest, trailing partial tile ]` of
    /// *static-range* `Repeat`s with **different inner trip counts**
    /// (the last tile is shorter when the extent is not a multiple of
    /// the block). Because this variant mirrors `Repeat`/`Sequence`
    /// structurally rather than parameterising one loop, that shape
    /// falls out naturally as **two sibling `Event::Loop`s with
    /// different `range`s** in the worker's event list — NOT one loop
    /// with a computed bound. A backend re-emits each sibling loop
    /// verbatim. This is the deliberate representation; do not collapse
    /// the siblings.
    ///
    /// ## Bound representation (AC#2 limitation — TASK-0160)
    ///
    /// `range` is a concrete [`Range<i64>`]. The symbolic loop-bound
    /// expression (e.g. `H - 1` un-evaluated) does NOT survive to here:
    /// `build_acfg` folds every loop bound to an `i64` constant when it
    /// constructs `ACFGNode::Repeat` (a non-const or overflowing bound
    /// is a typed `BuildAcfgError::NonConstLoopBound` /
    /// `OverflowingLoopBound`, not a panic — TASK-0179 / TASK-0398),
    /// so the un-evaluated expression no longer exists at the ACFG
    /// layer this projection reads. Carrying the symbolic form requires
    /// the lowering pass to stop folding — that is TASK-0160's
    /// (types/consts) territory, and TASK-0159 declares a dependency on
    /// it. A backend re-emits the loop with the concrete bound today;
    /// rendering `(16_i64 - 1_i64)` verbatim is unblocked only once
    /// TASK-0160 lands.
    Loop {
        iter_var: IterVar,
        range: Range<i64>,
        body: Vec<Event>,
        /// `Some` iff this loop is a strip-mined *inner* loop produced
        /// by `block_transform`; carries the per-occurrence
        /// absolute-index rebinding facts (TASK-0180). `None` for every
        /// source loop and every synthesised tile loop — those need no
        /// rebinding (the source loop iterates its real range; the tile
        /// loop's variable never appears in a body index). serde-default
        /// so the wire form stays backward-compatible (an old payload
        /// with no `block_tag` deserialises as `None`).
        #[cfg_attr(feature = "serde", serde(default))]
        block_tag: Option<BlockTag>,
        /// `Some` iff the user wrote `check loop V : latency_max=T
        /// [, on_violation=K]` whose `V` is this loop's source name AND
        /// this loop is the OUTER user-source loop (`block_tag ==
        /// None`); the codegen backend wraps the loop body in an
        /// `Instant::now()` measurement and a comparison against the
        /// threshold, panicking (default) / logging / counting on
        /// violation (PRD §6.3.5; TASK-0052.02). The projection pass
        /// `passes::inject_check_frames` populates this AFTER
        /// `acfg_to_events` so the `acfg_to_events` signature stays
        /// unchanged. serde-default keeps the wire form backward
        /// compatible (an old payload with no `check_frame`
        /// deserialises as `None`); see [`CheckFrame`].
        #[cfg_attr(feature = "serde", serde(default))]
        check_frame: Option<CheckFrame>,
        /// `Some(cond)` iff this loop is a source `for..until` early-exit
        /// loop (epic S4, TASK-0341.02.01.05.04): the bounded
        /// convergence/halt predicate, projected verbatim from
        /// [`crate::acfg::ACFGNode::Repeat::break_cond`] so the codegen
        /// backend can emit a runtime `break`. `None` for every ordinary
        /// fixed-iteration `for` loop AND for every synthesised tile /
        /// partition / strip-mined inner loop (`block_tag.is_some()`
        /// loops are compiler machinery, never a source convergence loop,
        /// so they never carry a break predicate).
        ///
        /// ## Why a third additive field on this SAME variant (not a new event)
        ///
        /// Same argument as `block_tag` / `check_frame` above: the break
        /// predicate is a per-loop-**occurrence** fact one-for-one with
        /// the `Repeat` node it projects from. A separate `Break` event
        /// would (a) duplicate the loop-identity join key (`iter_var`),
        /// (b) project ambiguously onto per-worker EventLists, and (c)
        /// force every existing consumer to acquire pairing logic. An
        /// optional annotation is the minimal, silent-sibling-safe change.
        ///
        /// ## ANALYSIS-INVISIBLE TO THE NET (keystone soundness)
        ///
        /// This field is purely a *codegen* contract; no analysis pass
        /// reads it. Boundedness is proved on the full-`range` unroll, and
        /// any early-exit prefix `0..k` (`k <= range.len()`) is a sub-trace
        /// of that bounded net, hence bounded a fortiori (epic keystone
        /// soundness argument, architect GO design-review cycle-254). The
        /// runtime break is emitted only in the single-worker sequential
        /// backend this slice (`pthreads-sync`); multi-worker / 7-backend
        /// break emit is a later slice (TASK-0341.02.01.08 / S7).
        ///
        /// `IrExpr` is the same node that already rides
        /// `ArgBinding::Scalar` / the `Repeat.break_cond` source, so the
        /// serde round-trip / determinism gate is already covered.
        /// serde-default keeps the wire form backward compatible (an old
        /// payload with no `break_cond` deserialises as `None`).
        #[cfg_attr(feature = "serde", serde(default))]
        break_cond: Option<IrExpr>,
    },
}

// Manual `Hash` for `Event` (see the enum's doc comment for why it is
// not derived: `Range<i64>` is not `Hash`). We hash a 1-byte
// discriminant tag plus the `Hash`-able fields, recursing through a
// `Loop` body. Fields that are not `Hash` (the `Range` in `Loop`) are
// hashed component-wise, exactly as [`IterTile`] does. Collisions
// across structurally distinct events are acceptable for a `Hash`
// impl; equality remains exact via the derived `PartialEq`.
impl Hash for Event {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Event::Fire {
                kernel,
                tile,
                bindings,
            } => {
                0u8.hash(state);
                kernel.hash(state);
                tile.hash(state);
                bindings.hash(state);
            }
            Event::Alloc { data, tile, region } => {
                1u8.hash(state);
                data.hash(state);
                tile.hash(state);
                region.hash(state);
            }
            Event::Push {
                dst,
                data,
                tile,
                seq,
            } => {
                2u8.hash(state);
                dst.hash(state);
                data.hash(state);
                tile.hash(state);
                seq.hash(state);
            }
            Event::Wait {
                src,
                data,
                tile,
                seq,
            } => {
                3u8.hash(state);
                src.hash(state);
                data.hash(state);
                tile.hash(state);
                seq.hash(state);
            }
            Event::Sync {
                participants,
                kind,
                sync,
            } => {
                4u8.hash(state);
                participants.hash(state);
                kind.hash(state);
                sync.hash(state);
            }
            Event::Free { data, tile } => {
                5u8.hash(state);
                data.hash(state);
                tile.hash(state);
            }
            Event::Loop {
                iter_var,
                range,
                body,
                block_tag,
                check_frame,
                break_cond,
            } => {
                6u8.hash(state);
                iter_var.hash(state);
                // `Range<i64>` is not `Hash`; hash the endpoints
                // component-wise (mirrors `IterTile`).
                range.start.hash(state);
                range.end.hash(state);
                // `BlockTag` is `Hash`-derivable; hash it so two
                // structurally distinct strip-mine occurrences (full
                // vs partial) don't collide in `HashSet` dedup paths.
                block_tag.hash(state);
                // `CheckFrame` is `Hash`-derivable; hash it so two
                // structurally identical loops with vs without a
                // check directive don't collide in `HashSet` paths
                // (TASK-0052.02).
                check_frame.hash(state);
                // `IrExpr` deliberately does NOT implement `Hash` (it
                // mirrors the AST, which doesn't — see the binding-type
                // Hash impls above). Hash only the 1-byte presence
                // discriminant, exactly as the `Range` endpoints are
                // hashed component-wise rather than via a `Range: Hash`:
                // two loops that differ ONLY in their break predicate may
                // collide in a `HashSet`, which is acceptable for a `Hash`
                // impl (equality stays exact via the derived `PartialEq`,
                // which DOES compare the full `IrExpr`). The break
                // predicate is a codegen-only annotation, so loops in the
                // dedup paths (which precede codegen) never actually carry
                // distinct predicates today; the discriminant suffices.
                break_cond.is_some().hash(state);
                // Recurse the body; length first so a prefix can't
                // collide with a different-length body.
                body.len().hash(state);
                for ev in body {
                    ev.hash(state);
                }
            }
        }
    }
}

impl Event {
    /// Construct a `Fire` with an explicit value binding.
    pub fn fire(kernel: KernelId, tile: IterTile, bindings: FireBinding) -> Self {
        Event::Fire {
            kernel,
            tile,
            bindings,
        }
    }

    /// Construct a `Fire` with **no** value binding
    /// (`FireBinding::none()`). For synthetic callers / tests that
    /// only care about firing order, not the values computed. The
    /// real projection pass (`petri_to_events`) uses [`Event::fire`]
    /// with the binding recovered from the ACFG.
    pub fn fire_bare(kernel: KernelId, tile: IterTile) -> Self {
        Event::Fire {
            kernel,
            tile,
            bindings: FireBinding::none(),
        }
    }

    /// Construct a rolled [`Event::Loop`] (TASK-0159). `body` is the
    /// already-projected per-worker event sequence for one iteration
    /// of the loop body; the backend replays it once per value of
    /// `iter_var` in `range`.
    pub fn loop_over(iter_var: IterVar, range: Range<i64>, body: Vec<Event>) -> Self {
        Event::Loop {
            iter_var,
            range,
            body,
            block_tag: None,
            check_frame: None,
            // Plain fixed-iteration loop: no early-exit predicate. The
            // `for..until` projection sets `Some` directly via the
            // struct literal in `petri_to_events` (TASK-0341.02.01.05.04).
            break_cond: None,
        }
    }

    /// Construct a strip-mined inner [`Event::Loop`] carrying its
    /// per-occurrence [`BlockTag`] (TASK-0180). Used by the projection
    /// (`petri_to_events`) when it walks an `ACFGNode::Repeat` that
    /// `block_transform` tagged as a strip-mined inner loop; the
    /// backend rebinds the loop variable from this tag alone.
    pub fn loop_over_tagged(
        iter_var: IterVar,
        range: Range<i64>,
        body: Vec<Event>,
        block_tag: BlockTag,
    ) -> Self {
        Event::Loop {
            iter_var,
            range,
            body,
            block_tag: Some(block_tag),
            check_frame: None,
            // A strip-mined inner loop is compiler machinery, never a
            // source `for..until` convergence loop, so it carries no
            // break predicate (TASK-0341.02.01.05.04).
            break_cond: None,
        }
    }
}
