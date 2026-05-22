---
id: TASK-0237
title: >-
  Extract shared test-helper crate for backend pipeline-lowering boilerplate
  (pre-Wave-B-2 hygiene)
status: Done
assignee: []
created_date: '2026-05-22 01:19'
updated_date: '2026-05-22 05:12'
labels:
  - tech-debt
  - test-infrastructure
  - M4
dependencies:
  - TASK-0236
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-23 review-gate D.2 finding (commit 8a5ee26): two ~30-line helper functions duplicate the same parse + lower + link + ACFG + block-transforms + partition-workers + sync/transfer inject + sidecar build pipeline across backend test suites:

- nucleus/backends/pthreads-sync/tests/multi_worker.rs::lower_multi_worker_check_schedule
- nucleus/backends/mp-tcp-bufsync/tests/check_frame_emit.rs::build_per_worker_partitioned

The same pattern also lives in pthreads-async/tests/skeleton.rs::lower_example_01_naive (the existing byte-identical test). And it will land a 4th time when Wave B-2 of TASK-0228 adds pthreads-async multi-worker emit-string tests.

The 4-way fanout (one per backend test suite) is the cost of NOT centralizing. Better to extract NOW (before Wave B-2 adds the 4th copy) than later (would require touching all 4 sites).

Scope:
- New crate nucleus/backends/test-common/ (or similar name; check workspace conventions).
- Public helper lower_for_test(algo_src: &str, sched_src: &str, opts: PipelineOpts) -> (BTreeMap<WorkerId, Vec<Event>>, NameTables, NameSidecar) where PipelineOpts toggles apply_partition_workers + inject_check_frames.
- All 3 existing duplication sites refactored to call the new helper.
- Wave B-2 of TASK-0228 uses the helper for the 4th call site naturally.

Why not now: the 3-way duplication is acceptable (cycle-20 had 2-way; cycle-23 adds 3rd). Architect's recommendation is to extract BEFORE the 4-way state lands, which means BEFORE Wave B-2 (or as the first step of Wave B-2).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 New shared test-helper crate exists with a single lower_for_test entry point.
- [x] #2 All 3 existing duplication sites (pthreads-sync multi_worker.rs, mp-tcp-bufsync check_frame_emit.rs, pthreads-async skeleton.rs) refactored to call the helper.
- [ ] #3 Wave B-2 of TASK-0228 uses the helper for its multi-worker tests rather than introducing a 4th duplicate.
- [x] #4 Workspace tests pass, clippy -D warnings clean, just e2e baseline preserved.
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 24 (2026-05-22): new test-common crate at nucleus/test-common landed; 3 duplicate helpers refactored to call the shared test_common::lower_for_test.

Files:
- NEW nucleus/test-common/Cargo.toml (deps: compiler only — no backend deps, avoids the circular arrow that pthreads-sync dev-dep test-common would create if test-common needed pthreads-sync).
- NEW nucleus/test-common/src/lib.rs: LowerForTestOpts + LowerForTestResult + lower_for_test() with 2 in-module smoke tests.
- nucleus/Cargo.toml workspace member registered.
- pthreads-sync/Cargo.toml: dev-dep test-common; tests/multi_worker.rs::lower_multi_worker_check_schedule refactored (drops ~25 LoC of inline pipeline + NameTables construction down to ~10 LoC of thin glue).
- mp-tcp-bufsync/Cargo.toml: dev-dep test-common; tests/check_frame_emit.rs::build_per_worker_partitioned refactored (same shape).
- pthreads-async/Cargo.toml: dev-dep test-common; tests/skeleton.rs::lower_example_01_naive refactored (default opts — no partition_workers/inject_check_frames since 01/naive is single-worker).

Key API design:
- lower_for_test returns the 5 raw reverse-name-table BTreeMaps + BTreeSet (NOT a pre-built NameTables struct), because NameTables lives in pthreads-sync and importing it would create a circular dep (pthreads-sync dev-deps test-common; test-common deps pthreads-sync = cycle).
- Each backend's call site composes its own NameTables from the 5 fields in a 5-line local block. Isolated per call site (not duplicated across the pipeline) and keeps the dependency graph clean.

AC#3 (Wave B-2 of TASK-0228 uses the helper for the 4th call site) is structurally satisfied — the helper EXISTS and is documented as the canonical lower-link-inject pipeline; Wave B-2's pthreads-async multi-worker tests will consume it. Not literally executed here because Wave B-2 hasn't landed yet.

Gate:
- cargo test --workspace: 578 / 0 / 2 (was 576; +2 new test-common smoke tests; the 3 refactored sites produce IDENTICAL behavior to pre-refactor — verified by all existing tests still passing).
- cargo clippy --workspace --all-targets -- -D warnings: clean (had to fix one doc-lazy-continuation lint when 'apply_partition_workers + inject_check_frames' got line-wrapped as a list-item continuation).
- just e2e: 36 / 29 / 0 / 7 baseline preserved.

Wave B-2 entry-criteria status:
- TASK-0226, TASK-0233, TASK-0234, TASK-0222 (AC#1/2), TASK-0236, TASK-0237 — ALL DONE.
- Wave B-2 has zero remaining preconditions; the actual codegen integration work is the only thing left.
<!-- SECTION:FINAL_SUMMARY:END -->
