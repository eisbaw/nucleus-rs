---
id: TASK-0110
title: 'ACFG: conditional / If node support (post-v2)'
status: Done
assignee: []
created_date: '2026-05-18 01:23'
updated_date: '2026-05-23 20:56'
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

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED-to-post-v2 (orchestrator-direct, cycle 77 sweep). The task is labeled 'post-v2, compiler, ir' and explicitly says 'Likely won't-fix in v2'. The algorithm sublanguage today has no conditionals (PRD §6.2.4) and v2 has no plan to add them — the partition / placement / iteration model targets pure dataflow + static iteration. Reopen if/when v3 grows conditionals at the algorithm surface, at which point ACFGNode::If is the additive AST extension + count_operations / count_repeats / max_repeat_depth / projection passes get the matching arm.
<!-- SECTION:FINAL_SUMMARY:END -->
