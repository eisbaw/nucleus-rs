---
id: TASK-0388
title: >-
  M8 — lift shared multi-worker MPI Plan into backend-common (de-dup
  mpi-blocking/mpi-nonblocking)
status: To Do
assignee: []
created_date: '2026-05-31 09:45'
labels:
  - M8
dependencies:
  - TASK-0046
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0046 copied the multi-worker Plan logic (host election, rank assignment, channel-id collection, barrier participant analysis, the non-whole-world-barrier + check-frame loud rejects, the render_worker_events walk) from mpi-blocking/src/multi_worker.rs into mpi-nonblocking/src/multi_worker.rs verbatim (mirror, to bound landing risk). The ONLY material difference between the two backends is the rendezvous prelude (blocking Send/Recv vs buffered Ibsend/Imrecv) + the buffered-send buffer attach. This duplication is a silent-sibling hazard. Lift the shared Plan into backend-common (precedent: tcp_plan/event_plan substrates, TASK-0244) with a thin per-backend prelude hook, so a fix to the Plan logic cannot skip one backend.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Shared multi-worker MPI Plan lives in backend-common; mpi-blocking and mpi-nonblocking both consume it via a thin prelude/transport hook
- [ ] #2 Byte-identity preserved: just check-mpi (mpi-blocking) and just check-mpi-nonblocking both still pass byte-exact after the lift
- [ ] #3 No duplicated Plan::build / render_worker_arm / collect_chan_peers across the two MPI backends
<!-- AC:END -->
