---
id: TASK-0164
title: Enable tier-2 MPI CI matrix row (M7)
status: Done
assignee: []
created_date: '2026-05-18 22:16'
updated_date: '2026-05-23 20:58'
labels:
  - infra
  - tooling
  - M7
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Re-enable the disabled tier-2 matrix include row in .github/workflows/ci.yml when M7 (mpi-blocking backend) lands. Per PRD §10.2 the row must run a compile-mandatory / run-best-effort recipe (localhost OpenMPI in CI), NOT 'just ci'. Referenced by a code comment in ci.yml (TASK-0057). Likely needs a self-hosted or OpenMPI-container runner — see TASK-0057 AC#6 notes.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 ci.yml tier-2 include row enabled for milestone M7
- [ ] #2 Row runs MPI compile + localhost-MPI best-effort run, not the tier-1 just ci gate
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED-to-M7 (orchestrator-direct, cycle 77 sweep). Labeled M7; description: 'Re-enable... when M7 (mpi-blocking backend) lands.' M7 not in progress yet. The CI matrix row is currently commented out in ci.yml with a TASK-0057 reference to this task as the re-enable trigger. Closing matches the cycle-77 deferred-closure pattern; reopen when M7 lands and the mpi-blocking backend is ready for CI.
<!-- SECTION:FINAL_SUMMARY:END -->
