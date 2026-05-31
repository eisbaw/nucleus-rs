---
id: TASK-0046
title: M8 — Tier 2 MPI non-blocking
status: To Do
assignee: []
created_date: '2026-05-17 23:08'
updated_date: '2026-05-31 08:01'
labels:
  - M8
  - backend
  - validation
dependencies:
  - TASK-0045.01
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0045.01 (mpi-blocking multi-worker arm, landed):

- TAG MAPPING (answers AC#5 "how to map SeqTag to MPI tags"): use the per-pair RENDEZVOUS ID (one per (DataId, SeqTag) cross-worker pair, the same numbering pthreads-sync slot_ids uses) as the MPI message TAG. This is LOAD-BEARING for value-correctness, not an optimization: a worker with multiple concurrent transfers to/from the SAME peer rank (e.g. 03-reduction host scatters a to 4 ranks + gathers partials from 4 ranks = 8 transfers) would have messages cross-match by send-order without distinct tags. mpiexec -n N with ALL ranks live exposes this; -n 1 hides it (memory 16-jacobi: deadlock-free != value-correct).

- SPMD REUSE PATTERN: emit ONE rank-dispatched binary (match world.rank()), reuse backend_common::multi_worker_walker::render_worker_events by emitting a per-rank PRELUDE of rendezvous wrapper types whose .push()/.wait() the walker targets (rendezvous_prefix). For mpi-nonblocking the wrappers wrap MPI_Isend/Irecv + a deferred MPI_Wait instead of blocking send/recv. The Fire/Loop arithmetic stays byte-shared (no drift). See nucleus/backends/mpi-blocking/src/multi_worker.rs.

- THE ASYNC SCHEDULES ARE YOURS: 05-stencil/distributed + distributed-2d request async/buffer=2/notify=event; mpi-blocking (sync-only) HARD-REJECTS them at the driver capability gate (check_schedule_compat). They are mpi-nonblocking targets. The deadlock motivation: mpi-blocking uses standard-mode MPI_Send which may block for messages above the eager limit (the schedule ordering targets the fully-async pthreads Slot model). MPI_Isend removes that hazard.

- HOST ELECTION / RANK ASSIGNMENT: host -> rank 0 (elect_host_from_worker_names, the shared helper), remaining used workers -> ranks 1..N in WorkerId order. Mirror it exactly (memory feedback-driver-must-mirror-backend-election-exactly).

- WHOLE-WORLD BARRIER ONLY so far: non-whole-world (host-excluding) barriers need MPI_Comm_split (TASK-0045.02, unproven); mpi-blocking rejects them loud. mpi-nonblocking inherits this gap unless 0045.02 lands first.

Depends on TASK-0045.01 (landed) + TASK-0045 (parent M7).
<!-- SECTION:NOTES:END -->
