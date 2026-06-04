---
id: TASK-0445
title: M8 mpi-nonblocking into the counted e2e matrix (async cells)
status: To Do
assignee: []
created_date: '2026-06-04 10:52'
labels:
  - M8
  - backend
  - validation
  - e2e
  - mpi
dependencies:
  - TASK-0444
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Extend TASK-0444's mpi tier to the M8 mpi-nonblocking backend. Today mpi-nonblocking value-correctness lives only in 'just check-mpi-nonblocking' (out-of-band, with the dual eager/forced-rendezvous arms). Add mpi-nonblocking to e2e-matrix.toml's mpi_backends tier and declare [[required]] cells for the async schedules it uniquely admits (05-stencil/distributed{,-2d}, 09-producer-consumer/pipelined, 11-game-of-life/pipelined) so they become COUNTED via 'just e2e-mpi'. The harness machinery already exists (--with-mpi tier gate, is_mpi_backend predicate, tight-tier scoping, mpiexec hard-fail probe); this is mostly a toml + capabilities decision. NOTE: the e2e run.sh path bakes default ranks = used-worker count; confirm mpi-nonblocking emits a harness-compatible run.sh (mpi-blocking does). Brings the matrix from 8 of 9 to 9 of 9 backends counted (only embedded-pattern then stays out, by design via renode-multimcu-gate).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 mpi-nonblocking added to mpi_backends; just e2e-mpi runs both mpi backends' declared cells byte-exact vs reference.bin
- [ ] #2 async-only schedules (05/distributed-2d w<->w halo, 09/pipelined host-excluding barrier, 11/pipelined) counted; dual eager/rendezvous concern noted (e2e uses default eager — forced-rendezvous stays in check-mpi-nonblocking)
- [ ] #3 default just e2e unchanged; --with-mpi still hard-fails without mpiexec
<!-- AC:END -->
