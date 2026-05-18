---
id: TASK-0023
title: e2e harness binary (nucleus-e2e)
status: To Do
assignee: []
created_date: '2026-05-17 23:05'
labels:
  - M1
  - tooling
  - validation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The single CLI entry point that runs the differential test matrix. Takes flags for example, schedule, backend; or runs full matrix when invoked bare. Justfile's e2e recipe calls this.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Crate 'e2e' produces a 'nucleus-e2e' binary.
- [ ] #2 Flags: --example <name>, --schedule <name>, --backend <name>, --milestone <id>; bare invocation runs the full matrix.
- [ ] #3 For each triple: compile via nucleus, run, diff against reference.bin, report pass/fail with timing.
- [ ] #4 Exit non-zero on any failure; print a final matrix summary.
- [ ] #5 Test: 'just e2e' runs and produces a green matrix at M1 (examples 1-3, naive only, pthreads-sync only).
- [ ] #6 Implementation notes record design questions (e.g. parallel vs sequential matrix execution; default for v2).
- [ ] #7 Implementation notes record honest limitations (e.g. timing reports only; no perf regressions tracked yet).
<!-- AC:END -->
