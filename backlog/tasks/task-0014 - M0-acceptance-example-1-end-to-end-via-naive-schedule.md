---
id: TASK-0014
title: 'M0 acceptance: example 1 end-to-end via naive schedule'
status: Done
assignee: []
created_date: '2026-05-17 23:03'
updated_date: '2026-05-18 02:16'
labels:
  - M1
  - validation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Milestone gate. Compile example 1 with the naive schedule via the nucleus binary; run the resulting program; verify output equals reference.bin.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Command 'nucleus build --algo examples/01-elementwise-add/prog.algo.nuc --sched examples/01-elementwise-add/schedules/naive.sched.nuc --backend pthreads-sync --out /tmp/out01' produces a runnable Rust program.
- [ ] #2 Running that program with input.bin produces output that bit-matches reference.bin.
- [ ] #3 CI runs this triple on every commit.
- [ ] #4 Test: the M0 acceptance script is invokable as 'just e2e --milestone M0'.
- [ ] #5 Implementation notes record the complete pipeline trace at M0 (which passes ran, what each emitted).
- [ ] #6 Implementation notes record honest limitations (e.g. pthreads-sync backend is the only one wired up; no Petri net IR yet).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Closed as subsumed by TASK-0020's e2e_example_01 test, which compiles example 01 + naive + pthreads-sync end-to-end with bit-identical output against reference.bin. See commit history for TASK-0020.
<!-- SECTION:NOTES:END -->
