---
id: TASK-0164
title: Enable tier-2 MPI CI matrix row (M7)
status: To Do
assignee: []
created_date: '2026-05-18 22:16'
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
