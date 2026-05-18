---
id: TASK-0055
title: Cross-worker transfer directive must be present (compile-error check)
status: To Do
assignee: []
created_date: '2026-05-17 23:10'
labels:
  - compiler
  - language
  - M1
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD §6.3.4: transfers crossing workers MUST have an explicit directive. Omission is a compile error. Implement the check; ensure error message names the offending data symbol and the producer/consumer worker pair.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Lowering pass detects cross-worker dataflow edges without a matching transfer directive.
- [ ] #2 Error message: 'data X flows from worker A to worker B; schedule has no transfer directive for X. Add transfer X : sync;'.
- [ ] #3 Intra-worker dataflow needs no directive; no event emitted.
- [ ] #4 Test: positive (all examples) compile; negative (synthetic schedule missing a transfer) produces the error.
- [ ] #5 Implementation notes record design questions (e.g. how to handle data that's intra-worker for some schedules and cross-worker for others; is this an error or a no-op).
- [ ] #6 Implementation notes record honest limitations (currently can't suggest a sensible default mode; just demands user be explicit).
<!-- AC:END -->
