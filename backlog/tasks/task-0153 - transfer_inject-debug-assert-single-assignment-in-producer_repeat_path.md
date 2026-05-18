---
id: TASK-0153
title: 'transfer_inject: debug-assert single-assignment in producer_repeat_path'
status: Done
assignee:
  - '@mark'
created_date: '2026-05-18 08:32'
updated_date: '2026-05-18 09:36'
labels:
  - M2
  - compiler
  - tech-debt
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
producer_repeat_path (TASK-0136 Pass B) assumes a unique single-assignment producer Operation per DataId and silently takes the first in walk order. v2 is single-assignment so this holds, but nothing enforces it; two writers of one DataId would mis-place the Push with no diagnostic. Add a debug-assert that produced count == 1. Also: record in TASK-0150 that the current structural loop-invariance model conservatively OVER-serialises (correctness-preserving, perf cost) so a future reader doesn't mistake it for a bug. Raised by mped-architect review of TASK-0136.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 debug_assert! that a DataId has exactly one producing Operation
- [x] #2 TASK-0150 notes updated: structural model over-serialises (safe), not a bug
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Added count_producers helper + debug_assert_eq!(count_producers(root,data),1) in splice_pushes_global before using producer_repeat_path's first-match result. Zero release cost; debug builds (cargo test) verify single-assignment across all example ACFGs (never trips). TASK-0150 notes appended: structural loop-invariance model conservatively over-serialises (safe perf ceiling), not a correctness bug. Full suite + clippy green.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Guard the single-assignment assumption in transfer_inject's producer lookup. producer_repeat_path silently takes the first Operation writing a DataId; a debug_assert now fails loud if there are several (a front-end/lowering bug that would mis-place the Push). Release builds unaffected. Also documented in TASK-0150 that the structural loop-invariance model only ever over-serialises (safe), so it is a precision/perf follow-up, not a correctness defect.
<!-- SECTION:FINAL_SUMMARY:END -->
