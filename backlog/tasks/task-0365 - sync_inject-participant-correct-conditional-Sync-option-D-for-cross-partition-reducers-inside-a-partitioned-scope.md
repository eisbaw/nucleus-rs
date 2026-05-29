---
id: TASK-0365
title: >-
  sync_inject: participant-correct conditional-Sync (option D) for
  cross-partition reducers inside a partitioned scope
status: To Do
assignee: []
created_date: '2026-05-29 21:33'
updated_date: '2026-05-29 21:34'
labels:
  - M6
  - compiler
  - sync_inject
  - tech-debt
  - forward-carried-from-TASK-0281
dependencies:
  - TASK-0281
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Forward-carried from TASK-0281 (the fail-loud guard). TASK-0281 turned the silent loss-of-synchronisation on an uncovered cross-partition cross-worker reducer (inside a partitioned scope) into a LOUD typed diagnostic SyncInjectError::UncoveredCrossPartitionReducer. This task is the deeper fix: instead of refusing, INSERT a participant-correct in-partition barrier (option D from the cycle-85 analysis) so such schedules lower correctly.

The hard part (carried from TASK-0268): a per-outer-iteration barrier inside a partitioned scope deadlocks under floor-with-spillover unequal per-worker iteration counts (e.g. 4 workers x 14 rows => 4/4/3/3) because workers with fewer iterations exit early and leave the barrier short of participants. Any inserted synchronisation must be participant-correct for the ACTUAL per-iteration participant set, which is genuinely hard. The fix shape named in TASK-0281 AC#2 is: replace the guard predicate with push_wait_pair_covers || partitioned_scope_covers, where partitioned_scope_covers evaluates whether the partition scope already provides equivalent once-per-iteration synchronisation for the specific participants present in that iteration.

Trigger: an M6+ schedule that actually exercises a cross-partition reducer. Until such a schedule exists this stays deferred; the TASK-0281 guard makes the latent case safe (loud) in the meantime. Reachability is (b): IR/ACFG-constructible only today (apply_partition_workers takes the UNION of body worker sets, so a coherent disjoint-subset reducer is not expressible as a sound .nuc schedule).

## Acceptance Criteria
<!-- AC:BEGIN -->
1. A synthetic (or real M6+) cross-partition reducer schedule lowers WITHOUT the TASK-0281 refusal — a participant-correct Sync is inserted (or an equivalent partition-scope coverage is proven).
2. The inserted synchronisation does NOT deadlock under floor-with-spillover unequal per-worker iteration counts (verified by the Petri-net deadlock checker AND e2e).
3. e2e baseline holds (currently 308/246/0/62/0); the existing TASK-0281 refusal tests in tests/sync_inject.rs are updated to assert the new accept-and-synchronise behaviour for the now-supported shape.

## Code reference
- src/passes/sync_inject.rs inject_in_sequence: the `else if` inside_partition branch returning SyncInjectError::UncoveredCrossPartitionReducer is the site this task lifts.
<!-- SECTION:DESCRIPTION:END -->

- [ ] #1 Cross-partition reducer lowers without the TASK-0281 refusal via a participant-correct Sync
- [ ] #2 Inserted sync does not deadlock under floor-with-spillover (Petri-net checker + e2e)
- [ ] #3 e2e baseline holds; TASK-0281 refusal tests updated to assert accept-and-synchronise
<!-- AC:END -->
