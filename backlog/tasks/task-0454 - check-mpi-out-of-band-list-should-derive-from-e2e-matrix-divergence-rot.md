---
id: TASK-0454
title: check-mpi out-of-band list should derive from e2e-matrix (divergence-rot)
status: To Do
assignee: []
created_date: '2026-06-07 07:35'
labels:
  - rigour
  - validation
  - backend
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architect P9 advisory (cycle-9): the check-mpi / check-mpi-nonblocking justfile recipes carry their OWN hardcoded example/schedule lists, separate from e2e-matrix.toml's [[required]] mpi cells. TASK-0453.09 broadened the matrix mpi-blocking arm from 3 to 8 cells but did NOT touch check-mpi (sound: e2e-mpi proves the matrix cells byte-exact, strictly stronger than check-mpi's out-of-band assertion). Residual: the two MPI coverage surfaces can drift. Cleanest fix: make check-mpi{,-nonblocking} derive their target list from the matrix rather than duplicating hardcoded specs. Low priority, defense-in-depth.
<!-- SECTION:DESCRIPTION:END -->
