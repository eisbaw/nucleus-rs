---
id: TASK-0119
title: 'Transfer-injection: support conflicting sync/async options on one directive'
status: Done
assignee: []
created_date: '2026-05-18 01:44'
updated_date: '2026-05-23 21:10'
labels:
  - M1
  - compiler
  - sched
  - follow-up
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Currently policy_from_directive() lets the LAST option win when a schedule writes 'transfer D : sync, async;'. The schedule lowering pass already flags this as a linker concern (grammar §2 note 7). Either reject in schedule lowering or in link, before transfer_inject runs. Filed so the silent last-wins behaviour doesn't become a maintenance pothole.
<!-- SECTION:DESCRIPTION:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed orchestrator-direct (cycle 77 continuation). Investigation showed the work was ALREADY DONE: sched/lower.rs:1156-1166 (lower_transfer) explicitly rejects 'transfer D : sync, async' as SchedLowerErrorKind::ConflictingTransferMode (the variant doc-comment at sched/ir.rs:519-531 cites TASK-0193 as the cycle that landed this). Test coverage exists: nucleus-compiler/tests/sched_lower.rs:1105 'negative_mutually_exclusive_transfer_sync_async'. The TASK-0119 description was effectively obsolete the moment TASK-0193 landed but was never closed. This cycle: (a) verified the reject is real (grep + line:1156-1166); (b) verified the test exists (sched_lower.rs:1105 — covers both the sync+async conflict AND the sync+sync repeat path); (c) fixed the recurring-doc-lie failure class — transfer_inject.rs:169-173 module doc-block still said 'No conflict detection between sync and async... the last option wins' which was FALSE post-TASK-0193; rewrote it to honestly say 'Conflict detection happens upstream at sched-lower' with the TASK-0193/TASK-0119 cite + the test name. Doc-lie failure class (cycles 73-76 each surfaced 3-4 from this class; cycle 77 surface continues to find them on every keystone-or-not touched module).
<!-- SECTION:FINAL_SUMMARY:END -->
