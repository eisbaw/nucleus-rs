---
id: TASK-0182
title: e2e harness flaky build-dir/CWD race under rapid/concurrent runs
status: To Do
assignee: []
created_date: '2026-05-19 02:44'
labels:
  - e2e
  - reliability
  - tooling
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
qa-test-runner advisory during TASK-0167 review (pre-existing, NOT a TASK-0167 regression — reproduced once in 3 back-to-back just e2e runs): 07-matmul/naive/pthreads-sync FAILed with `shell-init: getcwd: cannot access parent directories` + `ld.bfd: cannot open output file .../target/e2e-matrix/.../nuc_generated: No such file or directory`. Root: the harness builds all cells under one shared nucleus/target/e2e-matrix tree; rapid/concurrent invocations race on cwd/build-dir (a prior cells dir removed/recreated under another). Does NOT reproduce serially; the CI per-job matrix already serialises so CI is not currently exposed, but it undermines local reproducibility and any future parallel-cell execution (TASK-0023.01). Harden: per-cell isolated build dir (unique tmp per (example,schedule,backend) run), never chdir into a shared mutable tree, or an explicit lock. Add a concurrency stress test.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Each e2e cell builds/runs in an isolated dir; no shared-tree cwd race
- [ ] #2 A stress test runs e2e cells concurrently >=20x with zero infra-race failures
- [ ] #3 Serial just e2e remains byte-deterministic
<!-- AC:END -->
