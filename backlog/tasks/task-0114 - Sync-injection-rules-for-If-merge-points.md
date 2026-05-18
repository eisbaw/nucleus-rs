---
id: TASK-0114
title: 'Sync-injection: rules for If merge points'
status: To Do
assignee: []
created_date: '2026-05-18 01:34'
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
