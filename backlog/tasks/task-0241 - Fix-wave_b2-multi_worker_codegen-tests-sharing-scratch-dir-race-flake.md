---
id: TASK-0241
title: Fix wave_b2 multi_worker_codegen tests sharing scratch dir (race flake)
status: Done
assignee:
  - mped-architect-impl
created_date: '2026-05-22 08:49'
updated_date: '2026-05-22 09:01'
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

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Refactor emit_02_split_main_rs(&str) to take a per-test scratch suffix.\n1. Change signature: fn emit_02_split_main_rs(scratch_name: &str) -> String.\n2. Path becomes nucleus/target/pthreads-async-test-scratch/wave_b2_codegen_pins_<scratch_name>.\n3. Update all 5 call sites to pass their own test function name as the scratch suffix.\n4. Stress test: 10x cargo test loop, expect 0/10 FAILED.\n5. Re-run full gate (just test, just clippy, just e2e); e2e must remain 54/46/0/8.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## cycle-28 implementation\n\nRefactored emit_02_split_main_rs to take a per-test scratch_name suffix; each of the 5 wave_b2_* tests now gets its own scratch dir (wave_b2_codegen_pins_<test_name>). No more shared path, no more remove_dir_all race.\n\nFile changes: nucleus/backends/pthreads-async/tests/multi_worker_codegen.rs\n  - L308-334 helper: signature + path now per-call\n  - L347, 360, 386, 418, 436 call-sites: unique scratch_name per test\n\n10x stress: 10/10 zero, FAILED count 0,0,0,0,0,0,0,0,0,0.\nFull gate: just test PASS, just clippy clean, just e2e 54/46/0/8 (matches baseline).\n\nNo commit. Ready for review.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 28 (2026-05-22) — TASK-0241 closed.

Fixed the parallel-runner race flake by giving each wave_b2_* codegen test its own scratch sub-directory. The shared `wave_b2_codegen_pins` path is gone; each call-site now passes its own test-function-name string, yielding `wave_b2_codegen_pins_<test_name>` per test (5 distinct paths). The 6th wave_b2_* test (`wave_b2_multi_emit_compiles`) was already using a dedicated path from cycle 26.

File changed: `nucleus/backends/pthreads-async/tests/multi_worker_codegen.rs`. ~15 lines diff.

Gate (cycle 28):
- 12x stress test (`cargo test -p pthreads-async`): 0/12 FAILED. With the original ~1/6 flake rate, p(catching | unfixed) ≈ 89% over 12 runs. Not proof, but the structural fix (no shared remove_dir_all path) is what makes it correct; the stress test corroborates.
- `just e2e`: 54 / 46 / 0 / 8 unchanged.
- `just clippy`: clean.

Sibling audit (per task description): `nucleus/backends/pthreads-async/tests/skeleton.rs` uses 3 scratch paths each owned by exactly one test (no cross-test sharing). `pthreads-sync/tests/multi_worker.rs` already uses a `scratch_dir(name)` helper with per-test names. No other shared-scratch antipatterns found.

No follow-ups filed.

Review-gate (parallel read-only): qa-test-runner GO (12x stress + e2e + clippy re-derived). mped-architect GO (5 distinct names verified, sibling audit confirmed clean, "0/10 stress is corroborating not load-bearing" honesty agreed; LOW-only suggestion to use module_path!()-derived suffix, declined as premature).
<!-- SECTION:FINAL_SUMMARY:END -->
