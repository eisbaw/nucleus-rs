//! Typed validator for the [`Event`] contract
//! (TASK-0107, PRD §8.2 / §8.3 / §8.4).
//!
//! [`crate::event`] is deliberately a contract module: it defines the
//! event vocabulary but does *not* enforce semantic invariants. The
//! invariants are the responsibility of the scheduler / projection /
//! transfer-injection / sync-injection passes that *construct* events.
//! This module is the typed gate that asserts those passes' output
//! actually satisfies the contract.
//!
//! See also: TASK-0015's recorded honest limitation #4 ("No validation
//! in this module … filed TASK-0107").
//!
//! ## What is checked
//!
//! 1. **`Event::Push.dst != self_worker`** — a worker pushing to
//!    itself is meaningless / a structural bug. Per-worker.
//! 2. **Matched `(src, dst, data, tile, seq)` Push/Wait pairs** — for
//!    every `Push` on worker `S` carrying `(dst=D, data, tile, seq)`
//!    there must be a `Wait` on worker `D` carrying
//!    `(src=S, data, tile, seq)`, and vice versa. **Cross-worker**:
//!    needs the whole `{ WorkerId -> Vec<Event> }` map, not just one
//!    list.
//! 3. **`Event::Sync.participants` is non-empty** — an empty barrier
//!    is a no-op and almost certainly a bug. Per-event.
//! 4. **No overlapping `Alloc`** for the same `(data, tile)` on a
//!    worker — two live regions for the same datum slice = aliasing
//!    bug. Per-worker. **LATENT today**: `Event::Alloc` is not emitted
//!    by `passes::petri_to_events` (see `petri_to_events.rs:113`).
//!    This check is in place but cannot fire on any current live
//!    schedule. It will fire the moment Alloc codegen lands.
//! 5. **`Event::Free` is preceded by `Event::Alloc`** on the same
//!    worker for the same `(data, tile)`. Per-worker. **LATENT today**
//!    (same reason as (4): Alloc / Free are not emitted).
//! 6. **Cross-worker `Sync` participant-set agreement** — two or more
//!    [`Event::Sync`] events that share a [`SyncTag`] (the cross-worker
//!    barrier join key, TASK-0172) MUST carry identical `participants`
//!    sets. If a SyncTag is seen with >1 distinct participant set, the
//!    disjoint per-worker `EventList`s disagree on who is in the
//!    barrier. **Cross-worker** (needs the whole map). Recurses through
//!    `Event::Loop` bodies like every other invariant. TASK-0423.
//!
//! ## Recursion through `Event::Loop`
//!
//! `Event::Loop` is structure-preserving (TASK-0159); its `body`
//! `Vec<Event>` carries Push / Wait / Sync / Fire events the loop
//! replays. The validator recurses into every `body` it sees and
//! treats nested events as members of the enclosing worker's list. For
//! the per-worker per-event-position ordering used by Alloc/Free
//! reasoning we walk pre-order (loop header position is the position
//! of the `Event::Loop` itself, then body positions in order).
//!
//! ## What is NOT checked (gaps recorded honestly)
//!
//! - **`IterTile` well-formedness** (no `start >= end`, no duplicate
//!   `IterVar` axes). TASK-0015 deliberately left these undefined
//!   pending a PRD decision (recorded honest limitation #6). Out of
//!   scope here.
//! - **`Event::Loop` `range` non-emptiness**. An empty loop body /
//!   inverted range is faithfully projected; that is the projection's
//!   contract, not a validator concern.
//!
//! ## How this is wired
//!
//! [`validate_event_lists`] (the FULL surface, including invariant (2)
//! Push/Wait pair matching) is wired as a HARD production gate in the
//! driver's `cmd_build` (`driver/src/main.rs`, immediately before
//! `dispatch::dispatch_backend`, sibling of the accumulator cross-
//! check) — TASK-0422 (cycle-244). That is the FINAL per-worker
//! EventList that every backend consumes, after `acfg_to_events`,
//! `inject_check_frames`, the mp-* `host_mediation_inject` /
//! `host_data_relay_inject` re-routing, and `apply_safe_push_reorder`.
//! A violation returns `Result::Err` (typed-error / String channel —
//! never a panic; this project rejects panic-on-valid-input).
//!
//! [`validate_event_lists_strict_per_worker`] is *additionally* wired
//! as a `debug_assert!` at the output of
//! [`acfg_to_events`](crate::passes::petri_to_events::acfg_to_events)
//! (see `petri_to_events.rs`). It runs only invariants (1), (3), (4),
//! (5) — the *strictly per-worker* ones — because that earlier boundary
//! is ALSO reached by the driver's host-election PREVIEW projections on
//! pre-mediation ACFGs (`main.rs` ~484, ~537), where invariant (2) need
//! not yet hold. Promoting that `debug_assert` to the full validator
//! would fire on those valid intermediate states. The full validator
//! therefore belongs ONLY at the final consumption point above.
//!
//! Invariant (2) — Push/Wait pair matching — is the part of the full
//! surface that the strict subset deliberately omits.
//!
//! The HISTORICAL reason invariant (2) was once deferred entirely
//! (recorded in earlier revisions of this doc and
//! `passes::petri_to_events`) was that `transfer_inject`'s cross-scope
//! splicing limitation left the EventList with unmatched Wait events
//! for legitimate shipping programs (example 02-split-add: producer at
//! top level, consumer inside a `for`), so a hard assert of (2) would
//! have crashed debug builds on valid input. **That premise is now
//! STALE.** TASK-0136 (Pass A `hoist_invariant_waits` + Pass B
//! cross-scope `splice_pushes_global`) and the sibling TASK-0149 /
//! TASK-0151 / TASK-0364 work closed the gap; TASK-0428 (cycle-242)
//! empirically verified inv(2) on the projected (pre-mediation)
//! EventList for the ENTIRE example corpus (all 55 schedules — see
//! `tests/petri_to_events.rs::task0428_inv2_holds_for_entire_example_corpus`),
//! and TASK-0422.01 (cycle-243,
//! `driver/tests/task0422_01_inv2_post_mediation.rs`) verified it on
//! the POST-mediation EventList for all 4 mp-* backends (220 cells, 0
//! violations) — which is what made wiring the production gate safe.
//! The gate is COMPILE-time enforcement over whatever schedule is
//! built; the corpus sweeps prove it rejects zero shipping programs,
//! and the e2e bit-identical differential is defense-in-depth.
//!
//! Release builds skip the `debug_assert` entirely — the validator
//! has zero cost in production compilation. This is a contract-
//! internal check, not a user-facing diagnostic.

use std::collections::{BTreeMap, BTreeSet};

use crate::event::{DataId, Event, IterTile, SeqTag, SyncTag, WorkerId};

// --------------------------------------------------------------------
// Internal: IterTile sort key
// --------------------------------------------------------------------
//
// `IterTile` carries `Vec<(IterVar, Range<i64>)>`; `Range<i64>`
// deliberately does NOT implement `Ord` / `PartialOrd` (same
// long-standing std decision that made `IterTile` hand-roll its
// `Hash`). The validator needs deterministic iteration over Push /
// Wait keys (cross-worker closure, error emission order) and
// per-worker live-Alloc sets — both of which include an `IterTile`.
//
// Solution: canonicalise the tile to a `Vec<(u64, i64, i64)>`
// (IterVar id, start, end) for use as a sort key. The mapping is
// total, deterministic, and reversible-by-construction (we don't
// actually need to reverse it; the caller still holds the original
// `IterTile`).
type TileKey = Vec<(u64, i64, i64)>;

fn tile_key(t: &IterTile) -> TileKey {
    t.bounds
        .iter()
        .map(|(v, r)| (v.0, r.start, r.end))
        .collect()
}

// --------------------------------------------------------------------
// Error type
// --------------------------------------------------------------------

/// One structural violation of the [`Event`] contract.
///
/// Variants intentionally do NOT carry a source-position span: events
/// are post-lowering and have no source span (TASK-0015 limitation
/// #2). The diagnostic value is in the IDs themselves — a backend or
/// developer can cross-reference them against the
/// [`crate::sidecar::NameSidecar`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventValidationError {
    /// Invariant (1): a worker has an [`Event::Push`] whose `dst`
    /// equals its own [`WorkerId`]. Self-pushes are structurally
    /// meaningless.
    PushToSelf {
        worker: WorkerId,
        data: DataId,
        tile: IterTile,
        seq: SeqTag,
    },
    /// Invariant (2): a worker has an [`Event::Push`] that no other
    /// worker matches with a corresponding [`Event::Wait`] on the
    /// same `(src, dst, data, tile, seq)`.
    UnmatchedPush {
        src: WorkerId,
        dst: WorkerId,
        data: DataId,
        tile: IterTile,
        seq: SeqTag,
    },
    /// Invariant (2): a worker has an [`Event::Wait`] that no other
    /// worker matches with a corresponding [`Event::Push`] on the
    /// same `(src, dst, data, tile, seq)`.
    UnmatchedWait {
        src: WorkerId,
        dst: WorkerId,
        data: DataId,
        tile: IterTile,
        seq: SeqTag,
    },
    /// Invariant (3): an [`Event::Sync`] with an empty
    /// `participants` set.
    EmptySyncParticipants {
        /// The barrier identity (TASK-0172) of the offending Sync.
        sync: SyncTag,
    },
    /// Invariant (6): two or more [`Event::Sync`] events that share a
    /// [`SyncTag`] (the cross-worker barrier join key — TASK-0172) were
    /// seen with DIFFERENT `participants` sets. Every participant of one
    /// barrier must carry the same participant set, or the disjoint
    /// per-worker `EventList`s do not agree on who is in the barrier.
    /// **Cross-worker** (needs every worker's list — TASK-0423).
    SyncParticipantDisagreement {
        /// The barrier identity (TASK-0172) seen with >1 distinct
        /// participant set.
        sync: SyncTag,
    },
    /// Invariant (4): two [`Event::Alloc`] events for the same
    /// `(data, tile)` on the same worker without an intervening
    /// [`Event::Free`]. **LATENT today** (Alloc not emitted by
    /// `petri_to_events`).
    OverlappingAlloc {
        worker: WorkerId,
        data: DataId,
        tile: IterTile,
    },
    /// Invariant (5): an [`Event::Free`] with no preceding matching
    /// [`Event::Alloc`] on the same worker for the same `(data,
    /// tile)`. **LATENT today** (Free not emitted by
    /// `petri_to_events`).
    FreeWithoutAlloc {
        worker: WorkerId,
        data: DataId,
        tile: IterTile,
    },
}

impl std::fmt::Display for EventValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EventValidationError::PushToSelf {
                worker,
                data,
                tile,
                seq,
            } => write!(
                f,
                "worker {} has a Push targeting itself (data={}, tile.rank={}, seq={})",
                worker.0,
                data.0,
                tile.rank(),
                seq.0
            ),
            EventValidationError::UnmatchedPush {
                src,
                dst,
                data,
                tile,
                seq,
            } => write!(
                f,
                "Push on worker {} -> worker {} has no matching Wait \
                 (data={}, tile.rank={}, seq={})",
                src.0,
                dst.0,
                data.0,
                tile.rank(),
                seq.0
            ),
            EventValidationError::UnmatchedWait {
                src,
                dst,
                data,
                tile,
                seq,
            } => write!(
                f,
                "Wait on worker {} expecting from worker {} has no matching Push \
                 (data={}, tile.rank={}, seq={})",
                dst.0,
                src.0,
                data.0,
                tile.rank(),
                seq.0
            ),
            EventValidationError::EmptySyncParticipants { sync } => write!(
                f,
                "Event::Sync with sync_tag={} has empty participants",
                sync.0
            ),
            EventValidationError::SyncParticipantDisagreement { sync } => write!(
                f,
                "Event::Sync with sync_tag={} was seen with >1 distinct participant set \
                 across workers (cross-worker barrier participant sets must agree)",
                sync.0
            ),
            EventValidationError::OverlappingAlloc { worker, data, tile } => write!(
                f,
                "worker {} has overlapping Alloc for (data={}, tile.rank={}) \
                 with no intervening Free",
                worker.0,
                data.0,
                tile.rank()
            ),
            EventValidationError::FreeWithoutAlloc { worker, data, tile } => write!(
                f,
                "worker {} has Free for (data={}, tile.rank={}) with no preceding Alloc",
                worker.0,
                data.0,
                tile.rank()
            ),
        }
    }
}

impl std::error::Error for EventValidationError {}

// --------------------------------------------------------------------
// Validator — full surface (all 6 invariants)
// --------------------------------------------------------------------

/// Validate the per-worker EventList map against all six invariants
/// listed in the module docs. Returns every violation found, in
/// deterministic order (input map is a `BTreeMap`, walked by
/// ascending `WorkerId`; per-worker errors are emitted in event-
/// position order; cross-worker Push/Wait errors are emitted after
/// per-worker errors, sorted by `(src, dst, data, tile, seq)` via the
/// `BTreeMap` index; cross-worker Sync participant-disagreement errors
/// (invariant (6)) are emitted last, in ascending `SyncTag` order).
///
/// **Latent invariants**: (4) `OverlappingAlloc` and (5)
/// `FreeWithoutAlloc` are checked but cannot fire on any current
/// schedule because `passes::petri_to_events` does not emit `Alloc`
/// or `Free` (see `petri_to_events.rs:113`). They are in place so the
/// contract is documented in code, and so that future Alloc / Free
/// codegen lands against a green gate.
///
/// Pure function; never panics on user-reachable input.
pub fn validate_event_lists(
    by_worker: &BTreeMap<WorkerId, Vec<Event>>,
) -> Result<(), Vec<EventValidationError>> {
    let mut errors: Vec<EventValidationError> = Vec::new();

    // Cross-worker Push / Wait index. Keyed by the sort-friendly
    // tuple `(src, dst, data.0, tile_key, seq.0)` — `tile_key` is
    // the canonicalised `IterTile` (see module-level note: `Range<i64>`
    // is not `Ord`, so the raw `IterTile` cannot be a `BTreeMap` key).
    // The value carries the original `IterTile` verbatim so the
    // emitted error preserves it.
    //
    // The keying ensures deterministic closure-step iteration:
    // `BTreeMap` iterates by key, and the key is fully `Ord`.
    type Key = (u64, u64, u64, TileKey, u64);
    let mut pushes: BTreeMap<Key, IterTile> = BTreeMap::new();
    let mut waits: BTreeMap<Key, IterTile> = BTreeMap::new();

    // Cross-worker Sync participant-set agreement index (invariant (6),
    // TASK-0423). For each SyncTag we collect the SET of DISTINCT
    // participant sets seen across all workers (and all Loop nesting).
    // `BTreeSet<WorkerId>` is `Ord`, so `BTreeSet<BTreeSet<WorkerId>>`
    // is a valid set-of-distinct-sets; `len() > 1` means workers
    // disagree on the barrier membership for that tag. Iterated in
    // `BTreeMap` (SyncTag) order for deterministic emission.
    let mut sync_participants: BTreeMap<SyncTag, BTreeSet<BTreeSet<WorkerId>>> = BTreeMap::new();

    for (worker, list) in by_worker {
        check_per_worker(
            *worker,
            list,
            &mut errors,
            &mut pushes,
            &mut waits,
            &mut sync_participants,
        );
    }

    // Cross-worker closure: every Push must be matched by a Wait with
    // the same key, and vice versa. Iterate in `BTreeMap` order for
    // deterministic emission.
    for (k, tile) in &pushes {
        if !waits.contains_key(k) {
            errors.push(EventValidationError::UnmatchedPush {
                src: WorkerId(k.0),
                dst: WorkerId(k.1),
                data: DataId(k.2),
                tile: tile.clone(),
                seq: SeqTag(k.4),
            });
        }
    }
    for (k, tile) in &waits {
        if !pushes.contains_key(k) {
            errors.push(EventValidationError::UnmatchedWait {
                src: WorkerId(k.0),
                dst: WorkerId(k.1),
                data: DataId(k.2),
                tile: tile.clone(),
                seq: SeqTag(k.4),
            });
        }
    }

    // Cross-worker invariant (6): every SyncTag must carry one and the
    // same participant set across all workers (TASK-0423). >1 distinct
    // set => disagreement. Emitted after Push/Wait cross-worker errors,
    // in ascending SyncTag order (BTreeMap iteration) for determinism.
    for (sync, distinct_sets) in &sync_participants {
        if distinct_sets.len() > 1 {
            errors.push(EventValidationError::SyncParticipantDisagreement { sync: *sync });
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Validate only the strictly per-worker invariants: (1) no
/// self-push, (3) non-empty Sync participants, (4) no overlapping
/// Allocs, (5) no Free without Alloc.
///
/// Excludes invariant (2) — Push/Wait pair matching. NOTE: the
/// original reason (a `transfer_inject` cross-scope splicing
/// limitation leaving unmatched Waits for shipping programs) is STALE
/// as of TASK-0428 — inv(2) now holds across the entire example corpus
/// (see this module's docs). This subset is retained ONLY as the
/// `acfg_to_events` debug-assert, because that boundary precedes the
/// mp-tcp/uds mediation post-passes AND is reached by the driver's
/// pre-mediation host-election preview projections (`main.rs` ~484,
/// ~537) where inv(2) need not yet hold. The full validator
/// [`validate_event_lists`] (inv(2) included) is wired as the hard
/// production gate at the final EventList-consumption point in the
/// driver (`driver/src/main.rs` `cmd_build`, before
/// `dispatch::dispatch_backend` — TASK-0422 cycle-244). Use
/// [`validate_event_lists`] when you want inv(2) too.
///
/// Pure function; never panics on user-reachable input.
pub fn validate_event_lists_strict_per_worker(
    by_worker: &BTreeMap<WorkerId, Vec<Event>>,
) -> Result<(), Vec<EventValidationError>> {
    let mut errors: Vec<EventValidationError> = Vec::new();
    // Throwaway indices: invariants (1), (3), (4), (5) are all
    // per-worker, but `check_per_worker` does double duty by
    // populating these. We accept the small allocation cost rather
    // than duplicating the walker.
    let mut pushes: BTreeMap<(u64, u64, u64, TileKey, u64), IterTile> = BTreeMap::new();
    let mut waits: BTreeMap<(u64, u64, u64, TileKey, u64), IterTile> = BTreeMap::new();
    // Throwaway: `check_per_worker` populates this, but the strict
    // subset deliberately does NOT consume it. Invariant (6)
    // (SyncParticipantDisagreement) is CROSS-worker — it needs every
    // worker's lists at once — so it lives ONLY in the full
    // `validate_event_lists`, exactly like invariant (2). See module
    // docs (TASK-0423).
    let mut sync_participants: BTreeMap<SyncTag, BTreeSet<BTreeSet<WorkerId>>> = BTreeMap::new();
    for (worker, list) in by_worker {
        check_per_worker(
            *worker,
            list,
            &mut errors,
            &mut pushes,
            &mut waits,
            &mut sync_participants,
        );
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

// --------------------------------------------------------------------
// Per-worker walker
// --------------------------------------------------------------------

/// Walk one worker's event list once, registering:
/// - Invariant (1) self-push violations.
/// - Invariant (3) empty-Sync-participants violations.
/// - Invariant (4)/(5) Alloc/Free pairing violations (latent — see
///   module docs).
/// - The (src, dst, data, tile, seq) keys of every Push and Wait into
///   the cross-worker index for closure-check by the caller.
/// - Each `Event::Sync`'s `participants` set under its `SyncTag` into
///   the cross-worker `sync_participants` index, for invariant (6)
///   agreement-check by the caller (full validator only).
///
/// Recurses through `Event::Loop` bodies (the loop body's Pushes /
/// Waits / Allocs / Frees count as the enclosing worker's). Note:
/// Alloc/Free pairing across a loop boundary is conservatively
/// flattened pre-order — the loop body is treated as one straight
/// sequence. This is fine for the latent path (Alloc/Free unemitted)
/// and is the same conservativeness `petri_to_events` itself uses.
fn check_per_worker(
    worker: WorkerId,
    list: &[Event],
    errors: &mut Vec<EventValidationError>,
    pushes: &mut BTreeMap<(u64, u64, u64, TileKey, u64), IterTile>,
    waits: &mut BTreeMap<(u64, u64, u64, TileKey, u64), IterTile>,
    sync_participants: &mut BTreeMap<SyncTag, BTreeSet<BTreeSet<WorkerId>>>,
) {
    // Track currently-live `(DataId.0, tile_key)` Allocs on this
    // worker. `BTreeMap` value holds the original `IterTile` so
    // emitted errors are faithful.
    let mut live_allocs: BTreeMap<(u64, TileKey), IterTile> = BTreeMap::new();
    walk_events(
        worker,
        list,
        errors,
        pushes,
        waits,
        &mut live_allocs,
        sync_participants,
    );
}

#[allow(clippy::too_many_arguments)]
fn walk_events(
    worker: WorkerId,
    list: &[Event],
    errors: &mut Vec<EventValidationError>,
    pushes: &mut BTreeMap<(u64, u64, u64, TileKey, u64), IterTile>,
    waits: &mut BTreeMap<(u64, u64, u64, TileKey, u64), IterTile>,
    live_allocs: &mut BTreeMap<(u64, TileKey), IterTile>,
    sync_participants: &mut BTreeMap<SyncTag, BTreeSet<BTreeSet<WorkerId>>>,
) {
    for ev in list {
        match ev {
            Event::Push {
                dst,
                data,
                tile,
                seq,
            } => {
                // (1) self-push.
                if *dst == worker {
                    errors.push(EventValidationError::PushToSelf {
                        worker,
                        data: *data,
                        tile: tile.clone(),
                        seq: *seq,
                    });
                }
                // (2) cross-worker index. The src of a Push is the
                // worker recording it.
                pushes.insert(
                    (worker.0, dst.0, data.0, tile_key(tile), seq.0),
                    tile.clone(),
                );
            }
            Event::Wait {
                src,
                data,
                tile,
                seq,
            } => {
                // (2) cross-worker index. The dst of a Wait is the
                // worker recording it.
                waits.insert(
                    (src.0, worker.0, data.0, tile_key(tile), seq.0),
                    tile.clone(),
                );
            }
            Event::Sync {
                participants, sync, ..
            } => {
                // (3) non-empty participants.
                if participants.is_empty() {
                    errors.push(EventValidationError::EmptySyncParticipants { sync: *sync });
                }
                // (6) cross-worker participant-set agreement index
                // (TASK-0423): record this worker's view of the
                // participant set under the barrier's SyncTag. The
                // caller (full validator only) flags any tag whose
                // recorded set-of-distinct-sets has len > 1. Note the
                // empty set is still recorded here so an
                // empty-vs-nonempty disagreement also surfaces (the
                // empty case is additionally caught by (3)).
                sync_participants
                    .entry(*sync)
                    .or_default()
                    .insert(participants.clone());
            }
            Event::Alloc { data, tile, .. } => {
                // (4) overlapping Alloc on same (data, tile) with no
                // intervening Free.
                let key = (data.0, tile_key(tile));
                if let std::collections::btree_map::Entry::Vacant(e) = live_allocs.entry(key) {
                    e.insert(tile.clone());
                } else {
                    errors.push(EventValidationError::OverlappingAlloc {
                        worker,
                        data: *data,
                        tile: tile.clone(),
                    });
                }
            }
            Event::Free { data, tile } => {
                // (5) Free without preceding Alloc.
                let key = (data.0, tile_key(tile));
                if live_allocs.remove(&key).is_none() {
                    errors.push(EventValidationError::FreeWithoutAlloc {
                        worker,
                        data: *data,
                        tile: tile.clone(),
                    });
                }
            }
            Event::Loop { body, .. } => {
                // Structure-preserving (TASK-0159): recurse with the
                // SAME `live_allocs` state. A backend replaying the
                // loop sees the body events in order; flattening for
                // validation purposes is correct for the latent
                // Alloc/Free path and for the Push/Wait closure.
                walk_events(
                    worker,
                    body,
                    errors,
                    pushes,
                    waits,
                    live_allocs,
                    sync_participants,
                );
            }
            Event::Fire { .. } => {
                // Fire has no contract invariant in (1)-(5). Filed
                // as out-of-scope at module top.
            }
        }
    }
}
