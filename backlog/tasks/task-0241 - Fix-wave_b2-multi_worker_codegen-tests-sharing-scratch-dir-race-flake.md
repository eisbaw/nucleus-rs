---
id: TASK-0241
title: Fix wave_b2 multi_worker_codegen tests sharing scratch dir (race flake)
status: To Do
assignee: []
created_date: '2026-05-22 08:49'
labels:
  - flake
  - test
  - pthreads-async
dependencies:
  - TASK-0228
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0228 cycle-26 introduced 5 wave_b2_* tests in
nucleus/backends/pthreads-async/tests/multi_worker_codegen.rs that all
share the SAME fixed scratch directory
`nucleus/target/pthreads-async-test-scratch/wave_b2_codegen_pins/`
(lines 322-323 in the test file). Each test calls
`std::fs::remove_dir_all(&scratch)` then `emit(...)` — under cargo's
default parallel test runner, two tests can race on that path, one
test's `remove_dir_all` deleting files mid-flight under another
test's read, producing intermittent
`WriteFailed { kind: NotFound, path: ".../kernels.rs" }` failures.

Empirically reproduced this cycle (TASK-0229 verification): roughly
1 in 6 `just test` runs fails one of the wave_b2_* tests with the
NotFound on kernels.rs. The flake is pre-existing (reproduced on
clean master with TASK-0229's matrix changes stashed), NOT introduced
by TASK-0229. Filing it as the cycle-27 honest follow-up.

Fix shape (one of):
- Give each test its own per-test scratch sub-dir (e.g. include the
  test function name in the path) — mirrors the test_common pattern
  used elsewhere in the workspace.
- Or wrap the shared scratch in a `std::sync::Mutex` static so
  the tests serialise on that path.

The first approach is preferred — parallelism is a feature, the
shared path is the actual bug. Sibling tests in
pthreads-async/tests/skeleton.rs and pthreads-sync/tests/multi_worker.rs
should be audited for the same pattern (test_common::scratch_for_test
or similar helper already exists per cycle-25's TASK-0237).
<!-- SECTION:DESCRIPTION:END -->
