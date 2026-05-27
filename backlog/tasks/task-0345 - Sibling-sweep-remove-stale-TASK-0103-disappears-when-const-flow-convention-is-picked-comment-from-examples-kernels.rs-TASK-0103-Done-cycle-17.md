---
id: TASK-0345
title: >-
  Sibling sweep: remove stale TASK-0103 'disappears when const-flow convention
  is picked' comment from examples/*/kernels.rs (TASK-0103 Done cycle 17)
status: To Do
assignee: []
created_date: '2026-05-27 10:34'
labels:
  - examples
  - docs
  - comment-doc-lie
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architect cycle-199 P3.4 follow-up. TASK-0103 (PRD §6.2.2 example kernels.rs uses Nuc consts as Rust generics) was CLOSED cycle 17 (2026-05-22). However seven examples' kernels.rs files still carry the stale claim 'Single-source-of-truth violation (TASK-0103); disappears when the const-flow convention is picked':

- nuc-nucleus/examples/03-reduction/kernels.rs:54
- nuc-nucleus/examples/04-prefix-sum/kernels.rs:67
- nuc-nucleus/examples/05-stencil/kernels.rs:61
- nuc-nucleus/examples/06-separable-filter/kernels.rs:60
- nuc-nucleus/examples/07-matmul/kernels.rs:73
- nuc-nucleus/examples/08-histogram/kernels.rs:57
- nuc-nucleus/examples/10-wavefront/kernels.rs:43 (inherited from template in cycle 199)

The 'convention is picked' phrasing reads as 'future work' — but TASK-0103 closed by deciding the const-flow convention IS the Vec<i32> + runtime length-check pattern these examples already use. So the comment self-contradicts: it claims a violation that the closed task accepted as the canonical pattern.

This is a feedback-comment-doc-lie-recurring + feedback-silent-sibling-defect double-pattern: TASK-0103's closure didn't sweep the seven inherited comments, and every new example (most recently 10-wavefront) inherits the lie by template.

Fix: rewrite each occurrence to either (a) remove the comment entirely (since the Vec<i32> pattern IS the convention now) or (b) cite TASK-0103 as the DECISION, not a violation. Then check if the README's reference to 'Why Vec<i32>' section also needs updating.
<!-- SECTION:DESCRIPTION:END -->
