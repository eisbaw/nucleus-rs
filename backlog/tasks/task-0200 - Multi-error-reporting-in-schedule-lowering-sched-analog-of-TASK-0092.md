---
id: TASK-0200
title: Multi-error reporting in schedule lowering (sched analog of TASK-0092)
status: To Do
assignee: []
created_date: '2026-05-19 20:25'
labels:
  - M0
  - compiler
  - diagnostics
  - follow-up
dependencies:
  - TASK-0087
  - TASK-0196
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
lower_sched currently aborts on the first SchedLowerError. Mirror the algo-lowering multi-error follow-up TASK-0092 (and the parser multi-error pattern TASK-0080/0081/0087): collect ALL located SchedLowerError values in one pass so users see every schedule-semantic violation per compile cycle. The located substrate is already done (TASK-0196: SchedLowerError is a struct { kind, span: Option<Range<usize>> } with display_with_src), so each error already carries its own span — the work is to make lower_sched continue past the first violation and accumulate, then have the driver surface all (same header + one-line-per-error shape the parser driver block now uses). SCOPE = schedule LOWERING only (the schedule PARSER multi-error is TASK-0087 Done; the algo-lowering analog is TASK-0092 To Do). Filed as forward-carry from TASK-0087.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 lower_sched returns ALL SchedLowerError violations from one pass (not just the first), each retaining its own located span
- [ ] #2 Driver surfaces every schedule lowering error (header + one line each with its at L:C), mirroring the parser multi-error driver block
- [ ] #3 Deterministic: same SchedIR input -> identical error set+order (no HashMap/HashSet in the error path); full gate green (just test/e2e 30/26/0/4/0/determinism byte-identical x2/clippy --all-targets/ci); zero behaviour change for valid input
<!-- AC:END -->
