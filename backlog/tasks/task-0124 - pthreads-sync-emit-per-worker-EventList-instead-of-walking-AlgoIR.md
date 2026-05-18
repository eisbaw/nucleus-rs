---
id: TASK-0124
title: 'pthreads-sync: emit per-worker EventList instead of walking AlgoIR'
status: To Do
assignee: []
created_date: '2026-05-18 02:13'
labels:
  - M2
  - backend
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0020 codegen walks AlgoIR statements directly because the ACFG strips index expressions and the per-worker EventList (Fire/Alloc/Push/Wait/Sync/Free) is not yet produced (waits on TASK-0027). Once TASK-0027 lands, the backend should consume per-worker EventLists rather than the AlgoIR. This unifies tier-1 backends around the EventList contract (PRD §7.4 / §8.3) and eliminates the LinkedIR dependency from the emit signature.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 emit() signature changes to (per_worker_event_lists: BTreeMap<WorkerId, Vec<Event>>, kernels_rs_path, out_dir, sidecar_name_map).
- [ ] #2 Codegen no longer references AlgoIR/LinkedIR; only Event-typed input.
- [ ] #3 All existing tier-1 backends agree on this contract before M3 lands.
- [ ] #4 Depends on TASK-0027.
<!-- AC:END -->
