---
id: TASK-0045
title: M7 — Tier 2 MPI blocking
status: To Do
assignee: []
created_date: '2026-05-17 23:08'
labels:
  - M7
  - backend
  - validation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
First tier-2 backend: mpi-blocking via rsmpi. SPMD codegen (one binary dispatching on MPI_Comm_rank). Localhost MPI in CI. Examples 1-6 compile. PRD §11. Placeholder.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 backends/mpi-blocking/ crate lands with capabilities.toml.
- [ ] #2 SPMD codegen: one binary, rank-dispatched main; MPI_Init/MPI_Finalize wrap execution.
- [ ] #3 Localhost MPI (OpenMPI in CI container) runs examples 1-6 with bit-identical output.
- [ ] #4 Test: M7 acceptance includes a localhost mpiexec -n N run on each example.
- [ ] #5 Implementation notes record design questions (e.g. collective recognition deferred; point-to-point emitted everywhere).
- [ ] #6 Implementation notes record honest limitations (no real-cluster CI; CI is localhost-only at M7).
<!-- AC:END -->
