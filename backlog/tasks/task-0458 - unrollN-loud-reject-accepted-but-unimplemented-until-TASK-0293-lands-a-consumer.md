---
id: TASK-0458
title: >-
  unroll=N: loud reject (accepted-but-unimplemented) until TASK-0293 lands a
  consumer
status: To Do
assignee: []
created_date: '2026-06-09 22:00'
labels:
  - fail-fast
  - sched
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
From the 2026-06-09 architecture review (P1.3), independently verified: unroll=N is parsed (sched/parser.rs:585), divisibility-validated against block (sched/lower.rs:970-999), lowered to ResolvedLoopOption::Unroll (lower.rs:1055) — and consumed by NO pass (grep: only lower.rs touches the variant). A schedule author tuning unroll=8 silently gets nothing — a fail-fast violation and the exact silent-downgrade pattern the capability matrix exists to forbid elsewhere.

PRD 6.3.3 defers implementation to TASK-0293 (reopen on concrete LLVM-vs-DSL divergence evidence). This task is NOT blocked by TASK-0293 — it lands FIRST, making the surface honest until a consumer exists; TASK-0293 would then replace the reject with the real transform. Preferred: hard error naming the option as unimplemented and citing the deferral; alternative (record the decision here if taken): an unmissable warning. Keep the grammar production; thesis appendix B already states the option is inert — keep paper and compiler claims aligned whichever way this lands.

Note: an earlier filing of this task (TASK-0458) was archived because a wrong blocking dependency on TASK-0293 could not be removed via the CLI.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A schedule using unroll=N fails loudly (or the warn decision is recorded here) naming the unimplemented option and TASK-0293
- [ ] #2 Negative test pins the diagnostic
- [ ] #3 TASK-0293 cross-referenced both directions; thesis appendix B claim still accurate after the change
<!-- AC:END -->
