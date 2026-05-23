---
id: TASK-0108
title: Exhaustive trait-surface tests for event types
status: Done
assignee:
  - '@mark'
created_date: '2026-05-18 01:15'
updated_date: '2026-05-23 14:34'
labels:
  - M1
  - events
  - tests
  - followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0015's tests cover Debug-by-use, Clone-by-use, PartialEq, Hash, serde. They do not explicitly assert Send/Sync/Ord-on-newtypes/Default. Add compile-time assert_impl_all-style tests so a future derive deletion breaks the build. Reference: TASK-0015 notes.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 static_assertions / manual trait-bound tests for Send/Sync on Event and IterTile;Ord on each newtype id;Default on IterTile
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Cycle 69 (2026-05-23) — LANDED in-thread (no implementer subagent; small additive trait-assertion test).

Implementation: appended a 'Trait-surface exhaustiveness' section to nucleus/compiler/tests/event.rs (+57 LoC):
- helpers: const fn assert_send/assert_sync/assert_ord/assert_default (compile-time trait-bound checks; if the bound is unsatisfied the test fails to compile).
- test event_types_are_send_and_sync: Event, IterTile, FireBinding, ArgBinding, DataSlice, SyncKind all asserted Send + Sync.
- test newtype_ids_are_ord: KernelId, DataId, WorkerId, IterVar, SeqTag, SyncTag, Region all asserted Ord (PRD §10.1 determinism story relies on BTreeMap keying).
- test itertile_is_default: IterTile asserted Default + checks empty bounds.
- Also updated module header to retire the 'filed as follow-up' note and forward-point TASK-0107 for the *runtime* validator (which complements the *compile-time* contract this file pins).

Gate: cargo test -p compiler --test event = 37 passed / 0 failed (3 new + 34 existing). just clippy = clean under -D warnings. just e2e SKIPPED for this cycle: pure compile-time-assertion test, no code-generation surface affected. Determinism/xbackend gates also irrelevant for this change.

AC#1 (static_assertions / manual trait-bound tests for Send/Sync on Event and IterTile; Ord on each newtype id; Default on IterTile): MET. The acceptance criterion's exact list is covered + augmented (also Send+Sync on FireBinding/ArgBinding/DataSlice/SyncKind for completeness; if a future derive deletion drops Send from any of them, this test fails to compile).

Gotchas / honest limits:
- Did NOT add  crate dependency (the AC offers it as an option, not a requirement). Manual  is dep-free and identical in semantics. Project convention strongly prefers no-deps.
- Did NOT add a test that  is NOT Ord (it deliberately isn't, because Range<i64>: !Ord). The cycle-67 TASK-0107 work uses an internal TileKey canonicalisation around this; mentioned in event_validate.rs:110-117. A future change that adds Ord to IterTile would not be caught here, but TileKey would silently lose its raison d'être — not addressed by this task.
- Send/Sync are auto-impl'd; the assertion fires only if a future Event/IterTile field introduces a !Send or !Sync component (e.g. Rc, raw pointer). That's exactly the future-regression footgun the test prevents.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
TASK-0108 closed cycle 69. Compile-time trait-surface exhaustiveness for Event, IterTile, newtype IDs (Send+Sync, Ord, Default) is now pinned in nucleus/compiler/tests/event.rs. 37 tests pass; 3 new; module header updated to reflect coverage. No new deps. AC#1 met.
<!-- SECTION:FINAL_SUMMARY:END -->
