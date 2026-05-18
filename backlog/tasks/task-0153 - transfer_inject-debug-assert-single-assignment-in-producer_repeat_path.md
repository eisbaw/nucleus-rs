---
id: TASK-0153
title: 'transfer_inject: debug-assert single-assignment in producer_repeat_path'
status: To Do
assignee: []
created_date: '2026-05-18 08:32'
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
- [ ] #1 debug_assert! that a DataId has exactly one producing Operation
- [ ] #2 TASK-0150 notes updated: structural model over-serialises (safe), not a bug
<!-- AC:END -->
