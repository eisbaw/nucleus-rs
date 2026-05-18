---
id: TASK-0110
title: 'ACFG: conditional / If node support (post-v2)'
status: To Do
assignee: []
created_date: '2026-05-18 01:23'
labels:
  - post-v2
  - compiler
  - ir
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0016 ships ACFGNode with Operation, Repeat, Sequence, Sync, Xfer. The algorithm sublanguage today has no conditionals (PRD §6.2.4), so no If variant. If a future algorithm-side surface grows conditionals, add ACFGNode::If { cond, then_branch, else_branch } and update count_operations/count_repeats/max_repeat_depth + the projection passes accordingly. Likely won't-fix in v2.
<!-- SECTION:DESCRIPTION:END -->
