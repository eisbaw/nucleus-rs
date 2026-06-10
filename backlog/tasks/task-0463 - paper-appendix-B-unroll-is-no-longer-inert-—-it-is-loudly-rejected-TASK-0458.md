---
id: TASK-0463
title: >-
  paper appendix B: unroll is no longer inert — it is loudly rejected
  (TASK-0458)
status: Done
assignee: []
created_date: '2026-06-10 09:09'
updated_date: '2026-06-10 10:00'
labels: []
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0458 made unroll=N a loud sched-lowering reject (SchedLowerErrorKind::UnrollUnimplemented), no longer a silent no-op. paper/appendices/B-grammar.tex:154-156 still claims the unroll option "is accepted by the parser but is currently inert --- no transform pass consumes it". That is now INACCURATE/a lie: a schedule with unroll=N fails to compile with a typed diagnostic naming the option as accepted-but-unimplemented and citing TASK-0293. Update the sentence to say it is accepted by the parser but REJECTED at sched-lowering as accepted-but-unimplemented (a loud error, not a silent no-op) pending the deferred consumer (TASK-0293) — which also strengthens the no-silent-downgrade story the same paragraph makes two sentences later. NOTE: TASK-0458 could not make this edit itself because the paper/ tree was outside its file-ownership wave scope. AC: appendix B sentence updated to match the reject; grep paper/ for any other unroll-inert claim (chapters 05/06/09/11 mention unrolling only re: loop-structure preservation, not the schedule option — likely fine, but verify).
<!-- SECTION:DESCRIPTION:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
paper/appendices/B-grammar.tex unroll sentence updated: no longer "accepted but currently inert" — now documents the loud reject (accepted-but-unimplemented diagnostic) consistent with the no-silent-downgrade rule, production retained for the planned TASK-0293 consumer. PDF rebuilt green. Commit 32c3109.
<!-- SECTION:FINAL_SUMMARY:END -->
