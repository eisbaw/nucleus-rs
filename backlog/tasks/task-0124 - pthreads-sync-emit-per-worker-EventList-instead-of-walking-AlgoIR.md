---
id: TASK-0124
title: 'pthreads-sync: emit per-worker EventList instead of walking AlgoIR'
status: To Do
assignee:
  - '@mark'
created_date: '2026-05-18 02:13'
updated_date: '2026-05-18 09:41'
labels:
  - M2
  - backend
dependencies:
  - TASK-0156
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
BLOCKED — not implementable as specified. Investigated (parent-Claude, full backend read). AC#2 requires codegen to consume ONLY Event-typed input, but Event::Fire{kernel,tile} carries no per-firing argument/output value bindings and no index expressions (acfg_to_events hardcodes tile=empty). The current pthreads-sync backend produces value-correct, bit-identical code precisely BECAUSE it walks AlgoIR call/index expressions (render_main_rs over IrStmt). Switching to EventList-only without first extending the contract would either emit value-wrong code (breaks every bit-identical e2e) or smuggle AlgoIR back in (fake AC#2). Filed prerequisite TASK-0156 (Event carries per-Fire value bindings), which itself depends on TASK-0150 (index expressions through ACFG). TASK-0124 reset To Do with deps task-0150, task-0156. Not faking partial completion (CLAUDE.md: no workarounds, honest about blockers).
<!-- SECTION:NOTES:END -->
