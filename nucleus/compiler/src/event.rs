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
use std::ops::Range;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

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
// Event
// --------------------------------------------------------------------

/// The six event variants per PRD §8.3. Each event is the projection
/// of a transition firing onto the worker that owns it. A worker's
/// `EventList` is a `Vec<Event>` in execution order (TASK-0016+ owns
/// the actual emission; this module just defines the type).
///
/// Wire format (with `serde` feature on): externally tagged JSON by
/// default — `{"Fire": {"kernel": 0, "tile": {...}}}`. Backends and
/// golden tests can rely on this shape.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Event {
    /// Execute kernel `kernel` over iteration coordinates `tile`.
    /// `tile` is empty for top-level (non-iterated) firings.
    Fire { kernel: KernelId, tile: IterTile },

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
}
