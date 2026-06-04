---
id: TASK-0447
title: >-
  Bundle the manual mpi gate into one meta-recipe (incl. both required-coverage
  negative falsifiers)
status: To Do
assignee: []
created_date: '2026-06-04 23:58'
labels:
  - M7
  - M8
  - validation
  - mpi
  - test-hardening
  - tooling
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0446 review (mped-architect P2): the new required-coverage-check-negative-mpi arm is a WEAKER 'standing' guarantee than the tier-1 sibling — the tier-1 required-coverage-check-negative runs inside 'just ci' every cycle, but the mpi arm is out-of-default-ci (needs .#mpi) so it bites only when a human invokes it under .#mpi. Residual: a refactor severing the mpi-tier required-coverage hard-fail still ships green under 'just ci' plus a forgotten manual step. This matches how the entire M7/M8 mpi tier is treated (manual gate, like e2e-mpi/check-mpi/check-mpi-nonblocking), so it is consistent and honest — but the residual standing-bite gap should be tracked. PROPOSAL: add a meta-recipe (e.g. 'just mpi-gate') under .#mpi that runs, in one command: check-mpi + check-mpi-nonblocking + e2e-mpi + required-coverage-check-negative + required-coverage-check-negative-mpi (the falsifiers included), so 'run the mpi tier' is one command that includes its own negative arms. Alternatively/additionally: if an mpi-CI lane is ever added, it MUST run both required-coverage negative arms. LOW priority (the mpi gate is run rarely; the unit test + the standing recipe both exist; this is about discoverability + bundling, not a missing check).
<!-- SECTION:DESCRIPTION:END -->
