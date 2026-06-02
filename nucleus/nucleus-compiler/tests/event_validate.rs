//! Integration tests for the [`event_validate`] module (TASK-0107).
//!
//! Strategy: one minimal hand-built `BTreeMap<WorkerId, Vec<Event>>`
//! per [`EventValidationError`] variant family, asserting the
//! validator returns exactly that variant; plus one positive smoke
//! test for a small valid 2-worker EventList.
//!
//! `IterTile`s carry no axes (rank 0) throughout — the validator's
//! `(data, tile, seq)` join key works on equality, and the empty tile
//! is sufficient to exercise every code path. Adding axes would not
//! strengthen the test.

use std::collections::{BTreeMap, BTreeSet};

use nucleus_compiler::event::{DataId, Event, IterTile, SeqTag, SyncKind, SyncTag, WorkerId};
use nucleus_compiler::event_validate::{
    validate_event_lists, validate_event_lists_strict_per_worker, EventValidationError,
};

// --------------------------------------------------------------------
// Helpers
// --------------------------------------------------------------------

/// Empty tile — `IterTile::empty()` is the default and fine for
/// validator tests (the validator's join key is equality on the
/// whole tile; the empty tile is a valid canonical key).
fn t() -> IterTile {
    IterTile::empty()
}

/// Build a one-worker map.
fn one_worker(worker: u64, events: Vec<Event>) -> BTreeMap<WorkerId, Vec<Event>> {
    let mut m = BTreeMap::new();
    m.insert(WorkerId(worker), events);
    m
}

/// Build a two-worker map.
fn two_workers(a: (u64, Vec<Event>), b: (u64, Vec<Event>)) -> BTreeMap<WorkerId, Vec<Event>> {
    let mut m = BTreeMap::new();
    m.insert(WorkerId(a.0), a.1);
    m.insert(WorkerId(b.0), b.1);
    m
}

// --------------------------------------------------------------------
// Negative tests — one per invariant family
// --------------------------------------------------------------------

#[test]
fn neg_push_to_self() {
    // Worker 0 pushes to itself. The other worker is irrelevant.
    let map = one_worker(
        0,
        vec![Event::Push {
            dst: WorkerId(0),
            data: DataId(7),
            tile: t(),
            seq: SeqTag(42),
        }],
    );

    let errors = validate_event_lists(&map).expect_err("must reject self-push");
    assert!(
        errors.iter().any(|e| matches!(
            e,
            EventValidationError::PushToSelf {
                worker: WorkerId(0),
                data: DataId(7),
                seq: SeqTag(42),
                ..
            }
        )),
        "expected PushToSelf among errors, got {errors:?}"
    );
}

#[test]
fn neg_unmatched_push() {
    // Worker 0 pushes to worker 1, but worker 1 has no Wait. The
    // strict-per-worker validator must NOT flag this (invariant (2)
    // is cross-worker and excluded from the strict subset); the full
    // validator MUST.
    let map = two_workers(
        (
            0,
            vec![Event::Push {
                dst: WorkerId(1),
                data: DataId(3),
                tile: t(),
                seq: SeqTag(11),
            }],
        ),
        (1, vec![]),
    );

    // Full validator catches it.
    let errors = validate_event_lists(&map).expect_err("must reject unmatched Push");
    assert!(
        errors.iter().any(|e| matches!(
            e,
            EventValidationError::UnmatchedPush {
                src: WorkerId(0),
                dst: WorkerId(1),
                data: DataId(3),
                seq: SeqTag(11),
                ..
            }
        )),
        "expected UnmatchedPush among errors, got {errors:?}"
    );

    // Strict-per-worker validator does NOT catch it (by design — see
    // module docs). Documenting the carve-out in test form so the
    // exclusion is visible from the test suite.
    assert!(
        validate_event_lists_strict_per_worker(&map).is_ok(),
        "strict-per-worker validator must NOT flag unmatched Push"
    );
}

#[test]
fn neg_unmatched_wait() {
    // Worker 1 has a Wait expecting from worker 0, but worker 0 has
    // no Push.
    let map = two_workers(
        (0, vec![]),
        (
            1,
            vec![Event::Wait {
                src: WorkerId(0),
                data: DataId(5),
                tile: t(),
                seq: SeqTag(99),
            }],
        ),
    );

    let errors = validate_event_lists(&map).expect_err("must reject unmatched Wait");
    assert!(
        errors.iter().any(|e| matches!(
            e,
            EventValidationError::UnmatchedWait {
                src: WorkerId(0),
                dst: WorkerId(1),
                data: DataId(5),
                seq: SeqTag(99),
                ..
            }
        )),
        "expected UnmatchedWait among errors, got {errors:?}"
    );
}

#[test]
fn neg_empty_sync_participants() {
    let map = one_worker(
        0,
        vec![Event::Sync {
            participants: BTreeSet::new(),
            kind: SyncKind::Barrier,
            sync: SyncTag(17),
        }],
    );

    let errors = validate_event_lists(&map).expect_err("must reject empty Sync");
    assert_eq!(
        errors,
        vec![EventValidationError::EmptySyncParticipants { sync: SyncTag(17) }]
    );

    // Strict subset also catches this — invariant (3) is per-event,
    // not cross-worker.
    let errors_strict =
        validate_event_lists_strict_per_worker(&map).expect_err("strict must also reject");
    assert_eq!(errors_strict, errors);
}

#[test]
fn neg_sync_participant_disagreement() {
    // Invariant (6), TASK-0423. Two Sync events share SyncTag(5) but
    // carry DIFFERENT participant sets: worker 0 thinks the barrier is
    // {0,1}, worker 1 thinks it is {1,2}. The cross-worker validator
    // must flag SyncParticipantDisagreement { sync: SyncTag(5) }.
    let set_01: BTreeSet<WorkerId> = [WorkerId(0), WorkerId(1)].into_iter().collect();
    let set_12: BTreeSet<WorkerId> = [WorkerId(1), WorkerId(2)].into_iter().collect();
    let map = two_workers(
        (
            0,
            vec![Event::Sync {
                participants: set_01,
                kind: SyncKind::Barrier,
                sync: SyncTag(5),
            }],
        ),
        (
            1,
            vec![Event::Sync {
                participants: set_12,
                kind: SyncKind::Barrier,
                sync: SyncTag(5),
            }],
        ),
    );

    let errors = validate_event_lists(&map).expect_err("must reject disagreeing participant sets");
    assert!(
        errors
            .iter()
            .any(|e| matches!(e, EventValidationError::SyncParticipantDisagreement { sync: SyncTag(5) })),
        "expected SyncParticipantDisagreement {{ sync: 5 }} among errors, got {errors:?}"
    );

    // Cross-worker invariant: the strict-per-worker subset must NOT
    // flag it (mirrors the unmatched-Push carve-out — invariant (6) is
    // cross-worker and lives only in the full validator).
    assert!(
        validate_event_lists_strict_per_worker(&map).is_ok(),
        "strict-per-worker validator must NOT flag cross-worker Sync disagreement"
    );
}

#[test]
fn pos_sync_participants_agree() {
    // Non-tautology guard: two Sync with the SAME tag AND identical
    // participant sets must NOT produce a SyncParticipantDisagreement.
    // (Sanity: proves the check keys on DISTINCT sets, not on merely
    // seeing the tag more than once.)
    let set_01: BTreeSet<WorkerId> = [WorkerId(0), WorkerId(1)].into_iter().collect();
    let map = two_workers(
        (
            0,
            vec![Event::Sync {
                participants: set_01.clone(),
                kind: SyncKind::Barrier,
                sync: SyncTag(5),
            }],
        ),
        (
            1,
            vec![Event::Sync {
                participants: set_01,
                kind: SyncKind::Barrier,
                sync: SyncTag(5),
            }],
        ),
    );

    let result = validate_event_lists(&map);
    assert!(
        result
            .as_ref()
            .err()
            .map(|errs| !errs
                .iter()
                .any(|e| matches!(e, EventValidationError::SyncParticipantDisagreement { .. })))
            .unwrap_or(true),
        "agreeing participant sets must NOT yield SyncParticipantDisagreement, got {result:?}"
    );
    // The whole map is in fact clean (the two Syncs are the only
    // events), so the full validator returns Ok.
    assert!(
        result.is_ok(),
        "agreeing 2-worker barrier should validate clean, got {result:?}"
    );
}

#[test]
fn neg_overlapping_alloc() {
    // Two Allocs for the same (data, tile) on the same worker with
    // no intervening Free.
    //
    // LATENT INVARIANT (4): `petri_to_events` does not emit
    // `Event::Alloc` today (see `petri_to_events.rs:113`), so this
    // path is exercised ONLY by synthetic test input like this. The
    // check is in place against the day Alloc codegen lands.
    use nucleus_compiler::event::Region;
    let map = one_worker(
        0,
        vec![
            Event::Alloc {
                data: DataId(1),
                tile: t(),
                region: Region(0),
            },
            Event::Alloc {
                data: DataId(1),
                tile: t(),
                region: Region(0),
            },
        ],
    );

    let errors = validate_event_lists(&map).expect_err("must reject overlapping Alloc");
    assert!(
        errors.iter().any(|e| matches!(
            e,
            EventValidationError::OverlappingAlloc {
                worker: WorkerId(0),
                data: DataId(1),
                ..
            }
        )),
        "expected OverlappingAlloc among errors, got {errors:?}"
    );
}

#[test]
fn neg_free_without_alloc() {
    // A Free with no preceding Alloc on the same worker.
    //
    // LATENT INVARIANT (5): `petri_to_events` does not emit
    // `Event::Free` today.
    let map = one_worker(
        0,
        vec![Event::Free {
            data: DataId(2),
            tile: t(),
        }],
    );

    let errors = validate_event_lists(&map).expect_err("must reject Free without Alloc");
    assert_eq!(
        errors,
        vec![EventValidationError::FreeWithoutAlloc {
            worker: WorkerId(0),
            data: DataId(2),
            tile: t(),
        }]
    );
}

// --------------------------------------------------------------------
// Positive smoke — a small valid 2-worker EventList
// --------------------------------------------------------------------

#[test]
fn pos_smoke_2workers_push_wait_sync() {
    // Worker 0 pushes to worker 1; worker 1 waits from worker 0;
    // both participate in one barrier. This is the smallest
    // non-trivial valid topology and should validate clean under
    // BOTH the full and the strict-per-worker validators.
    let participants: BTreeSet<WorkerId> = [WorkerId(0), WorkerId(1)].into_iter().collect();
    let map = two_workers(
        (
            0,
            vec![
                Event::Push {
                    dst: WorkerId(1),
                    data: DataId(0),
                    tile: t(),
                    seq: SeqTag(1),
                },
                Event::Sync {
                    participants: participants.clone(),
                    kind: SyncKind::Barrier,
                    sync: SyncTag(0),
                },
            ],
        ),
        (
            1,
            vec![
                Event::Wait {
                    src: WorkerId(0),
                    data: DataId(0),
                    tile: t(),
                    seq: SeqTag(1),
                },
                Event::Sync {
                    participants,
                    kind: SyncKind::Barrier,
                    sync: SyncTag(0),
                },
            ],
        ),
    );

    assert!(
        validate_event_lists(&map).is_ok(),
        "full validator: {:?}",
        validate_event_lists(&map).err()
    );
    assert!(
        validate_event_lists_strict_per_worker(&map).is_ok(),
        "strict-per-worker validator: {:?}",
        validate_event_lists_strict_per_worker(&map).err()
    );
}

// --------------------------------------------------------------------
// Recursion into Event::Loop bodies — the validator must see Pushes
// and Waits buried inside `Event::Loop.body`
// --------------------------------------------------------------------

#[test]
fn neg_self_push_inside_loop_body() {
    // Same self-push violation, but nested inside a Loop. Asserts
    // the recursive walk-events path is hooked up.
    use nucleus_compiler::event::IterVar;
    let map = one_worker(
        0,
        vec![Event::Loop {
            iter_var: IterVar(0),
            range: 0..3,
            body: vec![Event::Push {
                dst: WorkerId(0),
                data: DataId(9),
                tile: t(),
                seq: SeqTag(123),
            }],
            block_tag: None,
            check_frame: None,
        }],
    );

    let errors = validate_event_lists(&map).expect_err("must reject nested self-push");
    assert!(
        errors.iter().any(|e| matches!(
            e,
            EventValidationError::PushToSelf {
                worker: WorkerId(0),
                data: DataId(9),
                seq: SeqTag(123),
                ..
            }
        )),
        "expected PushToSelf inside Loop body, got {errors:?}"
    );
}
