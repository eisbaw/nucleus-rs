---
id: TASK-0371
title: >-
  Driver/e2e negative regression test for partition-workers InsufficientWork
  (empty-band reject)
status: To Do
assignee: []
created_date: '2026-05-30 11:54'
labels:
  - e2e
  - partition
  - robustness
  - cycle-214-followup
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-214 architect P3 (TASK-0367 review). The empty-band reject (worker count > partitioned-dim length => PartitionError::InsufficientWork, driver exit 1, no panic) is pinned at the PASS level (compute_partition_bands + map_band_error unit tests in passes/common.rs + partition_workers.rs) and was verified ONCE manually at the driver level (exit 1, typed error, zero panic with RUST_BACKTRACE=1). But there is no DRIVER/e2e negative regression test, so if the driver error-mapping (.map_err in driver/src/main.rs:~388) later swallows or panics on the error, no test catches it. Add a driver-level (or e2e negative-arm) test that runs a >dim-worker 07-matmul schedule and asserts exit!=0 + typed-error message + NO panic. Low severity (the ?-propagated path is pass-level pinned).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A test (driver integration or e2e negative arm) emits a partition=workers schedule with worker count > partitioned-dim length and asserts: nonzero exit, the InsufficientWork typed-error message on stderr, and ZERO panic markers
<!-- AC:END -->
