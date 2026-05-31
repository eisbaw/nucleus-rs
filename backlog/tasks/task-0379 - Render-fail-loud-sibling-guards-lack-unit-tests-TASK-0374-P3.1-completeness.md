---
id: TASK-0379
title: Render fail-loud sibling guards lack unit tests (TASK-0374 P3.1 completeness)
status: To Do
assignee: []
created_date: '2026-05-31 01:30'
labels:
  - backend
  - gather
  - test
  - rigour
  - completeness
dependencies:
  - TASK-0374
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
COMPLETENESS follow-up to TASK-0374 (architect P3.1, gather review gate). TASK-0374 unit-pinned the 4 fail-loud arms of render_gather_index_load. Three OTHER render fail-loud guards remain unit-untested defense-in-depth: (1) render_int_expr Call-in-index arm, expr.rs:72 (UnsupportedFeature kernel call inside an integer index); (2) render_const_expr DataRef/Call-in-loop-bound arm, expr.rs:201-203; (3) render_flat_index own three guards, fire.rs:520/530/539. All three are DEEPER than the lowering reject (lower_index_expr rejects Call at lower.rs:1179; allow_gather=false rejects DataRef/Call in loop-bound position), so they are hard to reach from valid source today and are legitimate defense-in-depth, NOT source-reachable like the partial-rank arm. Add unit tests asserting each EmitError fires, mirroring render_gather_negative.rs, OR document precisely why each is structurally unreachable. Lower urgency than TASK-0374 since none is currently surface-reachable.
<!-- SECTION:DESCRIPTION:END -->
