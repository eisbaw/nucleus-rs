---
id: TASK-0237
title: >-
  Extract shared test-helper crate for backend pipeline-lowering boilerplate
  (pre-Wave-B-2 hygiene)
status: To Do
assignee: []
created_date: '2026-05-22 01:19'
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
- [ ] #1 New shared test-helper crate exists with a single lower_for_test entry point.
- [ ] #2 All 3 existing duplication sites (pthreads-sync multi_worker.rs, mp-tcp-bufsync check_frame_emit.rs, pthreads-async skeleton.rs) refactored to call the helper.
- [ ] #3 Wave B-2 of TASK-0228 uses the helper for its multi-worker tests rather than introducing a 4th duplicate.
- [ ] #4 Workspace tests pass, clippy -D warnings clean, just e2e baseline preserved.
<!-- AC:END -->
