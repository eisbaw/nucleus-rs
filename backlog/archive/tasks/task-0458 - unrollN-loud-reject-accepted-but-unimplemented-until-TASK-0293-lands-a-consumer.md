---
id: TASK-0458
title: >-
  unroll=N: loud reject (accepted-but-unimplemented) until TASK-0293 lands a
  consumer
status: To Do
assignee: []
created_date: '2026-06-09 21:59'
labels:
  - fail-fast
  - sched
dependencies:
  - TASK-0293
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
From the 2026-06-09 architecture review (P1.3), independently verified: unroll=N is parsed (sched/parser.rs:585), divisibility-validated against block (sched/lower.rs:970-999), lowered to ResolvedLoopOption::Unroll (lower.rs:1055) — and consumed by NO pass (grep: only lower.rs touches the variant). A schedule author tuning unroll=8 silently gets nothing — a fail-fast violation and the exact silent-downgrade pattern the capability matrix exists to forbid elsewhere.

PRD 6.3.3 defers implementation to TASK-0293 (reopen on concrete LLVM-vs-DSL divergence evidence); until then the surface must not lie. Preferred: hard error naming the option as unimplemented and citing the deferral; alternative (document the decision in this task if taken): a warning that cannot be missed. Keep the grammar production (appendix B of the thesis already states the option is inert — keep paper and compiler claims aligned whichever way this lands).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A schedule using unroll=N fails loudly (or the warn decision is recorded here) naming the unimplemented option and TASK-0293
- [ ] #2 Negative test pins the diagnostic
- [ ] #3 TASK-0293 cross-referenced both directions; thesis appendix B claim still accurate after the change
<!-- AC:END -->
