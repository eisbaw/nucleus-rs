---
id: TASK-0381
title: >-
  Complete render-layer fail-loud guard test inventory: classify_data_slice +
  ArgBinding::Nested (TASK-0379 P3 sibling sweep)
status: To Do
assignee: []
created_date: '2026-05-31 01:59'
labels:
  - backend
  - test
  - rigour
  - completeness
  - silent-sibling
dependencies:
  - TASK-0379
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
COMPLETENESS follow-up to TASK-0379 (architect P3, review gate). TASK-0379 + render_gather_negative.rs (TASK-0374) cover render_int_expr, render_const_expr, render_flat_index. FOUR render-layer fail-loud guards remain unit-untested, all in fire.rs and structurally sibling to ones already pinned: (1) fire.rs:376 ArgBinding::Nested -> UnsupportedFeature nested kernel call inside an argument expression (direct sibling of expr.rs:72 Call-in-index, the most glaring omission); (2) fire.rs:426 classify_data_slice missing-ResolvedType -> ContractGap (sibling of fire.rs:529); (3) fire.rs:438 classify_data_slice over-indexed -> UnsupportedFeature (sibling of fire.rs:539 rank-mismatch); (4) fire.rs:446 classify_data_slice scalar-data-indexed -> UnsupportedFeature. (fire.rs:346 no_std fixed-array mismatch is already covered by fire_args_nostd.rs:200 - NOT a gap.) Add unit tests mirroring render_guard_siblings.rs OR document precise unreachability. LOW urgency: all defense-in-depth, none source-reachable. Filing per silent-sibling discipline so the render-layer fail-loud inventory is genuinely complete.
<!-- SECTION:DESCRIPTION:END -->
