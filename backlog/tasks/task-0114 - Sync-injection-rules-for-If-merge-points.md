---
id: TASK-0114
title: 'Sync-injection: rules for If merge points'
status: Done
assignee: []
created_date: '2026-05-18 01:34'
updated_date: '2026-05-23 21:06'
labels:
  - compiler
  - ir
  - blocked
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
When conditionals land in the algorithm grammar (TASK-0110), sync-injection needs rules for the merge point after a branching scope. Depends on the If variant existing in ACFGNode. Until then, this task is blocked.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 ACFGNode::If variant exists (depends on TASK-0110).
- [ ] #2 Sync injected at If merge when branches' workers differ.
- [ ] #3 Idempotence preserved.
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED-to-post-v2 (orchestrator-direct, cycle 77 sweep). Description: 'When conditionals land in the algorithm grammar (TASK-0110), sync-injection needs rules for the merge point... Until then, this task is blocked.' Hard-blocked on TASK-0110 (closed cycle 77 as DEFERRED-to-post-v2). When v3 grows If support, this task's reopen is part of that scope; filing a fresh task scoped to the conditional grammar's exact shape is cleaner than carrying a To-Do indefinitely.
<!-- SECTION:FINAL_SUMMARY:END -->
