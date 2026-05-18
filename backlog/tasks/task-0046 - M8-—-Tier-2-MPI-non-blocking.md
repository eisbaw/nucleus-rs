---
id: TASK-0046
title: M8 — Tier 2 MPI non-blocking
status: To Do
assignee: []
created_date: '2026-05-17 23:08'
labels:
  - M8
  - backend
  - validation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Tier-2 milestone: mpi-nonblocking via MPI_Isend/MPI_Irecv/MPI_Wait. Schedules requiring async/buffered work over MPI. Examples 9, 11 over MPI. PRD §11. Placeholder.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 backends/mpi-nonblocking/ crate lands with capabilities.toml supporting async + buffer.
- [ ] #2 Generated code uses MPI_Isend/MPI_Irecv with explicit MPI_Wait sequenced per the EventList.
- [ ] #3 Examples 9, 11 run on localhost MPI with bit-identical output.
- [ ] #4 Test: M8 acceptance includes async + buffered schedules over MPI.
- [ ] #5 Implementation notes record design questions (e.g. MPI_Request lifetime in generated code; how to map SeqTag to MPI tags).
- [ ] #6 Implementation notes record honest limitations (no derived-type optimisation; one MPI_Type_contiguous per transfer at M8).
<!-- AC:END -->
