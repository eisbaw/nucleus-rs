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
//!   types it as `Set<WorkerId>`; concretely we want a `Set` that:
//!   - Has stable iteration order (for deterministic codegen and
//!     stable hashes across runs).
//!   - Implements `Hash` (so `Event: Hash`).
//!
//!   `HashSet` fails both. `BTreeSet<WorkerId>` satisfies both.
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
/// seq)`. Assigned monotonically per `(src, dst, data)` triple by the
/// scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct SeqTag(pub u64);

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
/// the tier-1 backends (pthreads-sync `render_call_arg`) currently
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
/// `EventList` is a `Vec<Event>` in execution order (TASK-0016+ owns
/// the actual emission; this module just defines the type).
///
/// Wire format (with `serde` feature on): externally tagged JSON by
/// default — `{"Fire": {"kernel": 0, "tile": {...}, "bindings":
/// {...}}}`. Backends and golden tests can rely on this shape.
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
    Sync {
        participants: BTreeSet<WorkerId>,
        kind: SyncKind,
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
    /// constructs `ACFGNode::Repeat` (it panics on a non-const bound),
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
            Event::Sync { participants, kind } => {
                4u8.hash(state);
                participants.hash(state);
                kind.hash(state);
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
            } => {
                6u8.hash(state);
                iter_var.hash(state);
                // `Range<i64>` is not `Hash`; hash the endpoints
                // component-wise (mirrors `IterTile`).
                range.start.hash(state);
                range.end.hash(state);
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
        }
    }
}
