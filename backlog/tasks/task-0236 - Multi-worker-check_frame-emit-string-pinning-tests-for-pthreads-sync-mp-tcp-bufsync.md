---
id: TASK-0236
title: >-
  Multi-worker check_frame emit-string pinning tests for pthreads-sync +
  mp-tcp-bufsync
status: To Do
assignee: []
created_date: '2026-05-22 00:50'
labels:
  - test-coverage
  - M4
  - backend
  - check-frame
dependencies:
  - TASK-0222
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-22 review-gate B.1 finding (commit 835c76a): the TASK-0222 template extraction is byte-transparent for the SHARED helpers (4 helpers; 12 inline writeln sites replaced by helper calls). The existing emit-string-pinning tests (pthreads-sync/tests/check_frame_codegen.rs 12 tests + mp-tcp-bufsync/tests/check_frame_emit.rs 4 tests) pin the SINGLE-WORKER emit paths end-to-end.

But the MULTI-WORKER emit paths (pthreads-sync/src/multi_worker.rs Plan::emit static+guard + render_worker_events Log+Count branches; mp-tcp-bufsync render_worker_program static+guard + render_worker_events Log+Count branches) have NO pinning tests. They are byte-transparent ONLY by shared-helper construction (one helper → multiple callers → drift can't differ between callers because they share the same source).

This is a real test-coverage gap. A future cycle that mistakenly inlines a writeln! back into one of the multi-worker sites (and lets the call-graph drift) would not be caught by today's tests.

Scope:
1. New test file nucleus/backends/pthreads-sync/tests/multi_worker_check_frame.rs that builds a multi-worker schedule with check_loop directives (Panic/Log/Count) and asserts the emitted multi_worker main.rs contains the expected static decl + guard local + per-thread branch shapes.
2. New test file nucleus/backends/mp-tcp-bufsync/tests/multi_worker_check_frame.rs with the same shape for the multi-process backend.
3. Reuse pthreads-sync's multi_worker.rs synthetic 2-worker test fixture (multi_worker_check_loop_panics_per_thread_with_loop_var_and_numbers at multi_worker.rs:multi_worker.rs) as the schedule template; extend it with on_violation=log and on_violation=count variants asserting emit-string-only (no run).

When Wave B-2 of TASK-0228 lands, the same test shape should also live at nucleus/backends/pthreads-async/tests/check_frame_emit.rs (covers TASK-0222 AC#3.c).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 nucleus/backends/pthreads-sync/tests/multi_worker_check_frame.rs pins static decl + guard local + per-thread Log/Count branch emit-strings for a synthetic 2-worker fixture.
- [ ] #2 nucleus/backends/mp-tcp-bufsync/tests/multi_worker_check_frame.rs does the same for the multi-process backend.
- [ ] #3 A regression that inlines a writeln! back into either multi_worker site (breaking the shared-helper invariant) trips the new tests.
<!-- AC:END -->
