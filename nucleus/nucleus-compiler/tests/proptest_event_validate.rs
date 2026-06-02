//! Property-based tests for the PRD §8.3 event-contract validator
//! [`nucleus_compiler::event_validate::validate_event_lists`]
//! (TASK-0429, cycle-246 hardening wave).
//!
//! ## Scope
//!
//! [`validate_event_lists`] is the production gate wired at
//! `driver/src/gate.rs` (TASK-0422/0423, cycles 242-245). It carries
//! two LOAD-BEARING contract claims in its docstring that, prior to this
//! file, had only 10 hand-built `#[test]` bite cases
//! (`tests/event_validate.rs`) and NO property test:
//!
//! 1. **Panic-freedom**: "Pure function; never panics on user-reachable
//!    input." The gate at `gate.rs` relies on this — a `panic!` there
//!    would crash the compiler on the very malformed input it is
//!    supposed to DIAGNOSE (the panic-not-diagnostic class this project
//!    rejects; memory `feedback-panic-not-diagnostic-recurring`).
//! 2. **Deterministic, sorted error emission**: the module doc + the
//!    gate's error message depend on a stable error `Vec` order
//!    (per-worker errors in event-position order by ascending
//!    `WorkerId`; cross-worker Push/Wait errors sorted by
//!    `(src,dst,data,tile,seq)`; SyncParticipantDisagreement last, by
//!    ascending `SyncTag`).
//!
//! These properties COMPLEMENT (do NOT replace) the curated bite cases
//! in `tests/event_validate.rs`: those pin specific error variants and
//! messages on minimal hand-built maps; these drive the SAME validator
//! with bulk-randomised, adversarial `BTreeMap<WorkerId, Vec<Event>>`
//! inputs (self-pushes, unmatched Push/Wait, empty Syncs, same-SyncTag
//! disagreeing participant sets, deeply-nested `Loop`s, Alloc/Free in
//! arbitrary order — the latter is NEW coverage; invariants (4)/(5) are
//! latent today, `petri_to_events` never emits Alloc/Free, so the
//! validator's Alloc/Free arms have never seen random input).
//!
//! ## Properties pinned
//!
//! - **P1 panic-freedom (PRIMARY).** For ANY generated map,
//!   `validate_event_lists(&m)` and `validate_event_lists_strict_per_worker(&m)`
//!   return (Ok or Err) WITHOUT panicking. A `proptest!` body that
//!   simply CALLS the fn and binds the result IS the panic-freedom
//!   property — the harness turns any panic into a shrinking test
//!   failure.
//! - **P2 determinism/purity.** `validate_event_lists(&m) ==
//!   validate_event_lists(&m)` — two calls yield an IDENTICAL `Vec`
//!   (same errors, same ORDER). Worker order is NOT permuted: the input
//!   is a `BTreeMap<WorkerId, _>`, which already canonicalises worker
//!   order, so a "permute workers" test would be trivially invariant.
//! - **P3 within-worker reorder invariance.** Rotating the events
//!   WITHIN each worker's `Vec` does NOT change the SET of the
//!   ORDER-INDEPENDENT error variants. This pins the soundness argument
//!   from TASK-0422.02 (`apply_safe_push_reorder`, a within-worker
//!   permutation, preserves inv(2)/inv(6)) as a GENERAL property.
//!
//!   ### P3 variant-scoping (a real semantic distinction, not a fudge)
//!
//!   The error variants split by their dependence on within-worker
//!   event ORDER:
//!   - ORDER-INDEPENDENT (compared under reorder):
//!     `UnmatchedPush`, `UnmatchedWait` (keyed by
//!     `(src,dst,data,tile,seq)` across workers — a `BTreeMap` index, so
//!     insertion order is irrelevant), `SyncParticipantDisagreement`
//!     (keyed by `SyncTag` over a set-of-distinct-sets),
//!     `EmptySyncParticipants` (per-event), and `PushToSelf`
//!     (per-event). Whether each fires depends only on the multiset of
//!     events on the worker, not their order.
//!   - ORDER-SENSITIVE (EXCLUDED from the reorder comparison):
//!     `OverlappingAlloc` / `FreeWithoutAlloc`. These DO depend on
//!     event order — a `Free` before its `Alloc` is `FreeWithoutAlloc`,
//!     but the reverse order is fine; two `Alloc`s without an
//!     intervening `Free` is `OverlappingAlloc`, order-dependent. They
//!     may LEGITIMATELY differ under reorder, so the P3 comparison
//!     filters them out. (These arms are latent in production but
//!     exercised here by the generator.)
//!
//! ## Honest-failure discipline
//!
//! If a property FAILS — a panic, a non-deterministic `Vec`, or a
//! reorder that changes the order-independent error set — that is a
//! REAL finding about a validator we have relied on across 5 cycles.
//! Do NOT weaken the property or `prop_assume!` the case away. Shrink
//! to the minimal failing input (proptest does this), report the
//! counterexample, and fix the validator or file a precise task. A
//! property test that reveals a bug is the BEST outcome of this cycle.
//!
//! ## Generator honest limits
//!
//! `worker_events_map()` draws `WorkerId`/`DataId`/`SeqTag`/`SyncTag`
//! from a SMALL domain (0..=3) and biases tiles toward the empty tile
//! (rank 0). This is DELIBERATE: with `u64`-wide ids the chance that a
//! `Push`'s `(src,dst,data,tile,seq)` key COLLIDES with some `Wait`'s
//! key — the only way inv(2)'s MATCHED (non-error) arm and inv(6)'s
//! same-`SyncTag`-DISAGREEING arm get exercised — is vanishingly small,
//! so the validator's interesting closure logic would almost never run.
//! Narrowing the domain makes matched Push/Wait pairs and SyncTag
//! collisions common. The trade-off: extreme/wide-id edge cases are
//! under-sampled here (the serde-fidelity generator in
//! `proptest_serde.rs` covers wide ids; this file targets the
//! validator's join/agreement logic). See the final task notes for an
//! empirical coverage assessment.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use proptest::prelude::*;

use nucleus_compiler::event::{
    DataId, Event, IterTile, IterVar, KernelId, Region, SeqTag, SyncKind, SyncTag, WorkerId,
};
use nucleus_compiler::event_validate::{
    validate_event_lists, validate_event_lists_strict_per_worker, EventValidationError,
};

// --------------------------------------------------------------------
// Domain knobs — small id space so inv(2)/inv(6) matched/colliding arms
// actually fire (see module-level "Generator honest limits").
// --------------------------------------------------------------------

/// Inclusive upper bound on the small id domain (WorkerId / DataId /
/// SeqTag / SyncTag). 0..=3 keeps Push/Wait key collisions and SyncTag
/// collisions common.
const ID_MAX: u64 = 3;

/// Proptest case count. Matched to the heavier `proptest!` blocks in
/// `proptest_petri.rs` style; the validator is cheap (no state-space
/// search), so we run a generous count for coverage of the matched
/// arms. Proptest is seeded by default, so runs are deterministic.
const CASES: u32 = 1024;

// --------------------------------------------------------------------
// Event generators — ADAPTED (copied, not shared) from
// proptest_serde.rs:417-496. proptest strategies are test-binary-local
// house style in this repo (each `tests/*.rs` is self-contained); they
// are not exported across test binaries. The id domains here are
// narrowed (see module docs); otherwise the shape mirrors the serde
// generator so the same Event space (incl. recursive Loop, Alloc/Free,
// empty Syncs) is covered.
// --------------------------------------------------------------------

fn small_id() -> impl Strategy<Value = u64> {
    0..=ID_MAX
}

fn worker_id() -> impl Strategy<Value = WorkerId> {
    small_id().prop_map(WorkerId)
}

fn iter_var() -> impl Strategy<Value = IterVar> {
    small_id().prop_map(IterVar)
}

/// Small inverted/degenerate-friendly `Range<i64>` over a tiny domain
/// so tiles collide (the validator joins on whole-tile equality).
fn range_i64() -> impl Strategy<Value = Range<i64>> {
    (0i64..=3, 0i64..=3).prop_map(|(a, b)| a..b)
}

/// Tile biased toward the empty (rank-0) tile so Push/Wait keys collide
/// often (the empty tile is the canonical key in the curated tests).
/// A minority of tiles carry 1-2 axes for diversity.
fn iter_tile() -> impl Strategy<Value = IterTile> {
    prop_oneof![
        3 => Just(IterTile::empty()),
        1 => prop::collection::vec((iter_var(), range_i64()), 1..=2).prop_map(IterTile::new),
    ]
}

/// Non-recursive `Event` leaf arms (every variant except `Loop`).
/// One `prop_oneof!` arm per variant; kept aligned with
/// `proptest_serde.rs::event_leaf` (which carries the break-to-update
/// completeness guard for the `Event` enum).
fn event_leaf() -> impl Strategy<Value = Event> {
    prop_oneof![
        // Fire { kernel, tile, bindings } — bindings empty (the
        // validator ignores Fire entirely; it is here for input
        // realism, not to exercise a validator arm).
        (small_id(), iter_tile()).prop_map(|(k, tile)| Event::Fire {
            kernel: KernelId(k),
            tile,
            bindings: Default::default(),
        }),
        // Alloc { data, tile, region } — exercises latent inv(4).
        (small_id(), iter_tile(), small_id()).prop_map(|(d, tile, r)| Event::Alloc {
            data: DataId(d),
            tile,
            region: Region(r),
        }),
        // Push { dst, data, tile, seq } — inv(1) self-push (dst==worker)
        // + inv(2) cross-worker matching.
        (worker_id(), small_id(), iter_tile(), small_id()).prop_map(
            |(dst, data, tile, seq)| Event::Push {
                dst,
                data: DataId(data),
                tile,
                seq: SeqTag(seq),
            }
        ),
        // Wait { src, data, tile, seq } — inv(2) cross-worker matching.
        (worker_id(), small_id(), iter_tile(), small_id()).prop_map(
            |(src, data, tile, seq)| Event::Wait {
                src,
                data: DataId(data),
                tile,
                seq: SeqTag(seq),
            }
        ),
        // Sync { participants, kind, sync } — 0..=4 participants INCL the
        // empty set (inv(3)); same-SyncTag-different-participants across
        // events drives inv(6). `SyncKind::Barrier` is the only variant
        // (mirrors proptest_serde.rs `Just(SyncKind::Barrier)`; the
        // completeness guard lives in that file).
        (
            prop::collection::btree_set(worker_id(), 0..=4),
            Just(SyncKind::Barrier),
            small_id(),
        )
            .prop_map(|(participants, kind, sync)| Event::Sync {
                participants,
                kind,
                sync: SyncTag(sync),
            }),
        // Free { data, tile } — exercises latent inv(5).
        (small_id(), iter_tile()).prop_map(|(d, tile)| Event::Free {
            data: DataId(d),
            tile,
        }),
    ]
}

/// Exhaustiveness teeth for [`Event`] (TASK-0429 review P3-2). NO
/// wildcard arm — adding a variant to `Event` breaks this match and
/// forces a matching `event_leaf`/`event_strategy` arm so the new
/// variant is actually fuzzed against the validator. Without this, a
/// future `Event` variant would silently go un-generated HERE while the
/// file still compiles green (the `feedback-silent-sibling-defect`
/// pattern): `proptest_serde.rs` carries the same guard for its own
/// copy of the strategy, but that guard only protects that file.
#[allow(dead_code)]
fn event_variant_completeness_guard(e: &Event) {
    match e {
        Event::Fire { .. } => {}
        Event::Alloc { .. } => {}
        Event::Push { .. } => {}
        Event::Wait { .. } => {}
        Event::Sync { .. } => {}
        Event::Free { .. } => {}
        Event::Loop { .. } => {}
        // INTENTIONALLY no `_ =>` arm: a new Event variant must
        // break-to-update the generator above.
    }
}

/// Full `Event` strategy: leaf arms plus the recursive `Loop` arm
/// (bounded depth ≤ 2). The loop body recurses into shallower events,
/// so the validator's `Event::Loop`-body recursion is exercised. The
/// loop's own `iter_var`/`range`/`block_tag`/`check_frame` are
/// validator-irrelevant, so set to simple/None values.
fn event_strategy() -> impl Strategy<Value = Event> {
    event_leaf().prop_recursive(2, 16, 3, |inner| {
        (iter_var(), range_i64(), prop::collection::vec(inner, 0..=3)).prop_map(
            |(iter_var, range, body)| Event::Loop {
                iter_var,
                range,
                body,
                block_tag: None,
                check_frame: None,
            },
        )
    })
}

/// The headline input: a `BTreeMap<WorkerId, Vec<Event>>` of 1..=4
/// workers (small WorkerId domain so cross-worker Push/Wait pairs
/// collide), each carrying 0..=4 arbitrary Events.
fn worker_events_map() -> impl Strategy<Value = BTreeMap<WorkerId, Vec<Event>>> {
    prop::collection::btree_map(
        worker_id(),
        prop::collection::vec(event_strategy(), 0..=4),
        1..=4,
    )
}

// --------------------------------------------------------------------
// P3 helpers — order-independent error subset + within-worker rotation
// --------------------------------------------------------------------

/// Retain only the ORDER-INDEPENDENT error variants (see module docs:
/// P3 variant-scoping). Excludes `OverlappingAlloc` / `FreeWithoutAlloc`
/// which legitimately depend on within-worker event order.
fn order_independent_errors(
    res: &Result<(), Vec<EventValidationError>>,
) -> BTreeSet<String> {
    let errs = match res {
        Ok(()) => return BTreeSet::new(),
        Err(e) => e,
    };
    errs.iter()
        .filter(|e| {
            !matches!(
                e,
                EventValidationError::OverlappingAlloc { .. }
                    | EventValidationError::FreeWithoutAlloc { .. }
            )
        })
        // Use the Display string as a set key. The variants are
        // PartialEq+Eq but not Ord/Hash; Display is deterministic and
        // order-stable. NOTE it renders `tile.rank()` (axis count), NOT
        // the full tile contents, so two errors differing ONLY in tile
        // contents at the same rank collapse to one key. That is
        // harmless for P3: such a collapse is itself independent of
        // within-worker event order, so the before/after sets stay
        // equal regardless. (P3 asserts reorder-invariance, not error
        // injectivity.)
        .map(|e| e.to_string())
        .collect()
}

/// Rotate each worker's event Vec left by one (a deterministic
/// within-worker permutation; identity for len ≤ 1). A genuine
/// reordering for len ≥ 2, which is the case P3 cares about.
fn rotate_within_workers(
    m: &BTreeMap<WorkerId, Vec<Event>>,
) -> BTreeMap<WorkerId, Vec<Event>> {
    m.iter()
        .map(|(w, evs)| {
            let mut v = evs.clone();
            if v.len() >= 2 {
                v.rotate_left(1);
            }
            (*w, v)
        })
        .collect()
}

// --------------------------------------------------------------------
// Properties
// --------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(CASES))]

    /// P1 (PRIMARY) — panic-freedom of the FULL validator. Binding the
    /// result is enough: the harness turns any panic on a generated
    /// input into a shrinking failure. A panic here would be a real
    /// defect (the gate at `driver/src/gate.rs` would crash instead of
    /// diagnosing). Honest-failure: do NOT weaken; report the shrunk
    /// counterexample and fix/file.
    #[test]
    fn p1_validate_never_panics(m in worker_events_map()) {
        let res = validate_event_lists(&m);
        // Trivial-but-real assertion: an Ok carries no errors, an Err
        // carries at least one. (The point is reaching here without a
        // panic.)
        match res {
            Ok(()) => {}
            Err(e) => prop_assert!(!e.is_empty()),
        }
    }

    /// P1 (PRIMARY) — panic-freedom of the strict per-worker validator
    /// (the `acfg_to_events` debug-assert path). Same input space.
    #[test]
    fn p1_validate_strict_never_panics(m in worker_events_map()) {
        let res = validate_event_lists_strict_per_worker(&m);
        match res {
            Ok(()) => {}
            Err(e) => prop_assert!(!e.is_empty()),
        }
    }

    /// P2 — determinism/purity. Two calls on the same input yield an
    /// IDENTICAL error `Vec` (same errors, SAME ORDER). Pins the stable
    /// emission order the gate's error message relies on. Worker order
    /// is not permuted (BTreeMap canonicalises it — see module docs).
    #[test]
    fn p2_validate_is_deterministic(m in worker_events_map()) {
        let a = validate_event_lists(&m);
        let b = validate_event_lists(&m);
        prop_assert_eq!(a, b);
    }

    /// P3 — within-worker reorder invariance. Rotating events within
    /// each worker's Vec does NOT change the SET of ORDER-INDEPENDENT
    /// errors (UnmatchedPush/UnmatchedWait/SyncParticipantDisagreement/
    /// EmptySyncParticipants/PushToSelf). OverlappingAlloc/
    /// FreeWithoutAlloc are EXCLUDED (order-sensitive; see module docs
    /// P3 variant-scoping). Pins the TASK-0422.02 safe-push-reorder
    /// soundness argument as a general property.
    #[test]
    fn p3_within_worker_reorder_preserves_order_independent_errors(
        m in worker_events_map()
    ) {
        let before = order_independent_errors(&validate_event_lists(&m));
        let rotated = rotate_within_workers(&m);
        let after = order_independent_errors(&validate_event_lists(&rotated));
        prop_assert_eq!(before, after);
    }
}
