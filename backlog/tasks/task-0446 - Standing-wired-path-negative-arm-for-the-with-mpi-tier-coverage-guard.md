---
id: TASK-0446
title: Standing wired-path negative arm for the --with-mpi tier coverage guard
status: In Progress
assignee:
  - '@mark'
created_date: '2026-06-04 10:52'
updated_date: '2026-06-04 23:41'
labels:
  - M7
  - validation
  - e2e
  - mpi
  - test-hardening
dependencies:
  - TASK-0444
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0444 added the --with-mpi tier gate to required_coverage_gaps, and a unit test (mpi_required_cell_unplanned_is_a_gap_under_with_mpi) proves the PURE function bites. But the WIRED path (run_inner -> required_coverage_gaps hard-fail) has no standing negative arm for the mpi tier — the existing required-coverage-check-negative recipe + NUC_REQUIRED_COVERAGE_NEGATIVE seam are tier-1-only by construction (the synthetic cell anchors on manifest.required.first(), a pthreads-sync cell; see the documented orthogonality note in maybe_inject_required_coverage_negative). So a future refactor could sever the mpi-tier coverage hard-fail with unit tests + e2e green. This is the SAME standing-bite gap TASK-0168 closed for tier-1. Low priority (the mpi gate is run rarely, the unit test covers the logic), but file it so it is not lost. Option: extend the injection seam to optionally anchor on an mpi backend + run under --with-mpi, or add a driver-call test seam.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 a standing check proves the --with-mpi required-coverage hard-fail path still bites (not just the pure-function unit test)
<!-- AC:END -->
