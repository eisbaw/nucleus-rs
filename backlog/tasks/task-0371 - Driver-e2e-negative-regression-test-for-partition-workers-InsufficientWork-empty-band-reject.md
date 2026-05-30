---
id: TASK-0371
title: >-
  Driver/e2e negative regression test for partition-workers InsufficientWork
  (empty-band reject)
status: Done
assignee:
  - '@claude'
created_date: '2026-05-30 11:54'
updated_date: '2026-05-30 19:00'
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
- [x] #1 A test (driver integration or e2e negative arm) emits a partition=workers schedule with worker count > partitioned-dim length and asserts: nonzero exit, the InsufficientWork typed-error message on stderr, and ZERO panic markers
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Clone the task0048_05_shim_rejection.rs subprocess pattern into a new driver integration test nucleus/driver/tests/task0371_partition_insufficient_work_reject.rs. Point --algo/--kernels at the real 07-matmul example files (no copy => no staleness); write an over-subscribed partition=workers schedule (17 workers w0..w16 on the i axis, N=16 rows => InsufficientWork L=16 < N=17) to a fresh tempdir; run the real nucleus build binary; assert (a) nonzero exit, (b) the InsufficientWork typed-error substring on combined stdout+stderr (strictly less than .. workers / cannot give every worker at least one row), (c) NO panicked marker. Gate: build/clippy/test/test-release/e2e. e2e totals must stay 322/265/0/57/0 (new test is a unit/integration add, not an e2e cell).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
IMPLEMENTED + CLOSED cycle-215 (orchestrator in-thread; commit 29f6d7f). New driver integration test nucleus/driver/tests/task0371_partition_insufficient_work_reject.rs clones the task0048_05_shim_rejection subprocess pattern: builds a 07-matmul partition=workers schedule over-subscribing the i axis (17 compute workers > N=16 rows), runs the real nucleus binary, asserts exit!=0 + the InsufficientWork typed-error substrings (strictly less than / 17 workers / cannot give every worker at least one row) + NO panicked marker. --algo/--kernels point at the real 07-matmul files (no stale copy); the invalid schedule is written to a tempdir (not committed under examples/). EMPIRICALLY VERIFIED real driver output: exit 1, message via the driver partition-workers error: wrap. PARALLEL REVIEW GATE both GO: qa-test-runner (build/clippy clean; dev 1143/0/3 release 1142/0/3; e2e 322/265/0/57/0 reproduced 2x) + mped-architect (ran the CONTROL case 16-workers -> builds OK exit 0, proving over-subscription is the SOLE failure cause and guard-removal would be caught; pipeline ordering apply_partition_workers BEFORE capabilities/codegen is load-bearing and holds; no overclaim). Two P3 observations both no-action (panicked-substring limitation matches house style; pipeline-order dependency documented at source).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
DONE cycle-215. AC#1 MET: driver-level negative regression test pins the partition=workers InsufficientWork empty-band reject (exit!=0 + typed InsufficientWork message + zero panic), closing the gap where only the PASS-level unit tests + a one-time manual driver check covered it. Gate green (dev 1143/0/3, release 1142/0/3, e2e 322/265/0/57/0 x2); both review agents GO incl. an empirical control-case proof the test pins the guard and cannot pass for the wrong reason.
<!-- SECTION:FINAL_SUMMARY:END -->
