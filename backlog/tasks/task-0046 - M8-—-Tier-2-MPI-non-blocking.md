---
id: TASK-0046
title: M8 — Tier 2 MPI non-blocking
status: To Do
assignee: []
created_date: '2026-05-17 23:08'
updated_date: '2026-05-31 08:42'
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

ORCHESTRATOR SETUP (post TASK-0045.01 landing) — implementation design + the deep trap:

REUSE (≈80% from mpi-blocking multi_worker.rs): SPMD match-rank dispatch, host election (rank 0), tag=rid discipline, the per-rank prelude pattern, check-mpi gate structure, capability gate. New crate nucleus/backends/mpi-nonblocking/ mirrors mpi-blocking shape; capabilities.toml sets supports_async=true + supports_buffer=true + notify includes event (so 05-stencil/distributed{,-2d} STOP being capability-rejected and become this backend targets, plus examples 9/11 per AC#3).

THE DEEP TRAP (AC#5 MPI_Request lifetime) — budget the whole cycle around it:
- Isend(&buf) returns a Request immediately but the SEND BUFFER must stay alive + unmutated until a matching Wait/Test on the request completes. In generated Rust a naive `mpi_<rid>.push(data.clone())` drops the temporary clone at end-of-statement => use-after-free BEFORE the network reads it => silent corruption that is TIMING-DEPENDENT (a byte-exact -n N run may PASS by luck on localhost eager). This is why this task needs fresh, careful context, not a tail-end cycle.
- Design: the non-blocking rendezvous must OWN the buffer + request until Wait. push = Isend storing (Request, owned buffer) in a scope-lived holder; the producer Waits on its own send-requests before the buffer is dropped/reused (end of scope, or before re-push to same slot). Receiver: Irecv into an owned buffer + Wait before first use. rsmpi 0.8: immediate_send / immediate_receive_into return a Request scoped to a StaticScope or a `mpi::request::scope`; study mpi-0.8.1 request.rs (Scope/WaitGuard/RequestCollection) — the borrow checker enforces buffer-outlives-request via the scope lifetime, which is the SAFE path but constrains codegen structure.
- VERIFICATION must defeat the timing-luck: run check-mpi-nonblocking with LARGER message sizes (above the eager limit, forcing rendezvous protocol) and/or valgrind/MPI correctness checker if available, not just localhost -n N byte-exact. A pure eager-size byte-exact pass does NOT prove buffer-lifetime correctness (memory: deadlock-free != value-correct generalizes to use-after-free != detected).

STEP PLAN: (1) crate+capabilities(async,buffer)+driver dispatch+help+registered-list. (2) multi_worker emit reusing mpi-blocking, swapping the rendezvous prelude for Isend/Irecv+scoped-Wait holders. (3) check-mpi-nonblocking gate: examples 9,11 + the now-admitted async distributed schedules, at -n N AND a large-message variant. (4) honest limits + MPI_Request-lifetime design notes. (5) unit shape tests + the buffer-lifetime reasoning pinned.
<!-- SECTION:NOTES:END -->
