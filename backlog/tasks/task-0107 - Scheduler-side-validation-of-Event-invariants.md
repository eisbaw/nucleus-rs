---
id: TASK-0107
title: Scheduler-side validation of Event invariants
status: In Progress
assignee:
  - '@mped-architect-impl'
created_date: '2026-05-18 01:15'
updated_date: '2026-05-23 14:11'
labels:
  - M1
  - events
  - validation
  - followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0015 deliberately left validation out of the type module. The scheduler must enforce: Push.dst != self_worker, matched (src,dst,data,tile,seq) Push/Wait pairs, non-empty Sync.participants, no overlapping Alloc/Free for the same (data, tile), Free preceded by Alloc on same worker. Reference: PRD §8.2, §8.3, §8.4, TASK-0015 notes.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Scheduler rejects an EventList that violates any documented Event invariant;each invariant has a typed error variant and a negative test
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Implementation plan (cycle 65):

1. New module nucleus/compiler/src/event_validate.rs (sibling of event.rs).
   - pub enum EventValidationError with 6 variants:
     - PushToSelf { worker, data, tile, seq }
     - UnmatchedPush { src, dst, data, tile, seq }
     - UnmatchedWait { src, dst, data, tile, seq }
     - EmptySyncParticipants { sync }
     - OverlappingAlloc { data, tile }     (LATENT — Alloc not emitted today)
     - FreeWithoutAlloc { worker, data, tile } (LATENT — Free not emitted today)
   - Derive Debug, Clone, PartialEq, Eq + impl Display + impl std::error::Error.
   - pub fn validate_event_lists(by_worker: &BTreeMap<WorkerId, Vec<Event>>)
     -> Result<(), Vec<EventValidationError>>; returns ALL errors in
     deterministic order (sorted: WorkerId then event-position). Pure
     function, never panics.
   - Recurses into Event::Loop bodies (which can carry Push/Wait/Sync).

2. Wire as a debug_assert.
   - CRITICAL gotcha: petri_to_events module docs (lines 162-175) state
     that transfer_inject's cross-scope splicing limitation leaves
     legitimate ACFGs with unmatched Wait events on the consumer side
     (e.g. 02-split-add). A debug_assert at acfg_to_events output
     would fire on real, currently-shipping programs. Per task body's
     'alternative — your call' clause: do NOT wire invariant (2) as a
     hard assert at acfg_to_events output today. Wire the validator,
     but compose it as TWO functions:
     - validate_event_lists(): all 6 invariants (the pure typed surface).
     - validate_event_lists_strict_per_worker(): runs only invariants
       (1), (3), (4), (5), and (6 -- self push) — the strictly-per-worker
       checks that hold across the transfer_inject limitation. Use this
       as the debug_assert at acfg_to_events output.
   - Document this explicitly at the wire site AND in the validator
     module header. Invariant (2) is exposed via the public function
     but not asserted; callers (backend codegen, future fix to
     transfer_inject) can opt in.

3. Negative tests in nucleus/compiler/tests/event_validate.rs.
   - One test per variant family (6 tests).
   - One positive smoke (2-worker valid EventList).
   - Use the same synthetic-ACFG helper pattern as petri_to_events.rs.

4. Honest doc-comment policy.
   - Module header explicitly lists which invariants are LATENT today
     (Alloc/Free not emitted by petri_to_events.rs:113) and which is
     EXPOSED-BUT-NOT-ASSERTED (Push/Wait matching, due to
     transfer_inject limitation).
   - No comment claims 'validates all 6 invariants at the wire site'.

5. Gate.
   - nix develop --command just test          — 0 failures, 7 new tests added.
   - nix develop --command just clippy        — clean under -D warnings.
   - nix develop --command just e2e           — 88/70/0/18 unchanged.
   - nix develop --command just determinism-check-negative — OK.
   - nix develop --command just xbackend-check-negative    — OK.

6. Commit + notes.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 67 implementation (DONE-pending-commit)

### Files

- nucleus/compiler/src/event_validate.rs (new, 493 LoC) — module with EventValidationError + validate_event_lists + validate_event_lists_strict_per_worker.
- nucleus/compiler/tests/event_validate.rs (new, 348 LoC) — 6 negative tests (one per variant family), 1 positive smoke (2-worker Push/Wait/Sync), 1 nested-Loop recursion test (8 tests total — exceeds AC).
- nucleus/compiler/src/lib.rs — pub mod + pub use re-exports.
- nucleus/compiler/src/passes/petri_to_events.rs — wired debug_assert!(validate_event_lists_strict_per_worker(&out).is_ok()) at end of acfg_to_events.

### Gate (all green)

- just test: all crates pass; +8 new tests in event_validate.rs.
- just clippy: clean under -D warnings (one map_entry lint corrected mid-flight).
- just e2e: 88 / 70 / 0 / 18 — UNCHANGED from baseline.
- just determinism-check-negative: OK (70 of 88 cells perturbed).
- just xbackend-check-negative: OK (1 of 16 corruptions detected).

### Design choices made (with rationale on each)

1. **Two-function surface** (validate_event_lists + validate_event_lists_strict_per_worker). The full validator covers all 6 invariants; the strict-per-worker subset omits invariant (2) (Push/Wait pair matching). Rationale: petri_to_events.rs module docs (lines 162-175) state that transfer_inject has a known cross-scope splicing limitation that leaves legitimate EventLists with unmatched Wait events today (example 02-split-add). Asserting (2) at the acfg_to_events boundary as debug_assert would crash debug builds on real input. Strict subset is the safe debug_assert site; full validator is exposed for backend codegen and the future transfer_inject fix.

2. **TileKey canonicalisation** for use as a BTreeMap key. IterTile contains Range<i64>, which deliberately does NOT implement Ord (std long-standing decision; same one that made IterTile hand-roll Hash). Canonical encoding to Vec<(u64, i64, i64)> gives a deterministic Ord-able join key without leaking the canonicalisation outside the module. Original IterTile is retained as the BTreeMap value so emitted errors stay faithful.

3. **Recursion through Event::Loop bodies.** Required since TASK-0159 made the projection structure-preserving (Push/Wait/Sync/Alloc/Free can be buried in Loop bodies). walk_events recurses with the same live_allocs state; pre-order flatten is conservative for the latent Alloc/Free path and correct for the Push/Wait closure. Added neg_self_push_inside_loop_body test to pin the recursion.

4. **Deterministic error order.** BTreeMap iteration (WorkerId ascending) gives deterministic per-worker order. Cross-worker Push/Wait closure errors emit in BTreeMap key order = (src.0, dst.0, data.0, tile_key, seq.0) — fully deterministic. Matches the convention in link.rs (returns ALL errors, sorted).

### Latent invariants (honestly flagged in doc comments)

Invariants (4) OverlappingAlloc and (5) FreeWithoutAlloc cannot fire on any current schedule because passes::petri_to_events.rs:113 does not emit Event::Alloc / Event::Free. Doc comments on the variants AND on the module header explicitly say so. Tests still exercise the code path with synthetic input so the check has known-correct behaviour the day Alloc/Free codegen lands.

### Carve-out invariant (also flagged)

Invariant (2) Push/Wait pair matching is implemented but EXPLICITLY NOT asserted at acfg_to_events output. Doc comments on validate_event_lists AND on the wire-site debug_assert explain why (transfer_inject limitation). The negative test neg_unmatched_push asserts BOTH that the full validator catches it AND that the strict-per-worker validator does NOT — pinning the carve-out in test form.

### Gotchas for next implementer

- If transfer_inject's cross-scope splicing limitation gets fixed (separate task), upgrade the debug_assert at petri_to_events.rs to validate_event_lists (the full surface) — then invariant (2) becomes load-bearing at the projection boundary.
- The validator does NOT check cross-worker Sync participant agreement (TASK-0172's SyncTag join). Filed for follow-up at the module header. Today's invariant (3) only checks per-event non-emptiness.
- IterTile well-formedness (start<end, no duplicate IterVar axes) is out of scope per TASK-0015 honest limitation #6.
<!-- SECTION:NOTES:END -->
