---
id: TASK-0464
title: >-
  Add 06-separable-filter/distributed{,2} as mpi-blocking matrix cells (restore
  check-mpi coverage dropped by TASK-0454)
status: To Do
assignee: []
created_date: '2026-06-10 09:13'
labels:
  - rigour
  - validation
  - backend
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0454 made check-mpi derive its cell list from e2e-matrix.toml mpi-blocking [[required]] cells. Net coverage delta: it GAINED 07/08/15/16/17 distributed (-n5, stronger) but DROPPED two genuine multi-worker cells that were ONLY in the old hardcoded recipe list and are not in the matrix: 06-separable-filter/distributed (n=5) and 06-separable-filter/distributed2 (n=5). (It also dropped 04/05/06 naive -n1 smoke, which is the weakest signal -- -n1 hides Send/Recv ordering bugs per the matrix comment -- not worth restoring.) Correct fix under the new single-source design: add the two 06-separable-filter distributed schedules as [[required]] mpi-blocking cells (milestone M7) in nuc-nucleus/e2e-matrix.toml; they then flow into BOTH just e2e-mpi (counted differential) AND just check-mpi automatically. Verify byte-exact under mpiexec --oversubscribe -n 5 first (run check-mpi after adding to confirm the derivation picks them up). e2e-matrix.toml was read-only for the TASK-0454 agent (being restructured in another wave), hence this follow-up.
<!-- SECTION:DESCRIPTION:END -->
