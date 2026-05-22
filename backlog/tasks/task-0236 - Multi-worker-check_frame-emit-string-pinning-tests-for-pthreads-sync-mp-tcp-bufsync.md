---
id: TASK-0236
title: >-
  Multi-worker check_frame emit-string pinning tests for pthreads-sync +
  mp-tcp-bufsync
status: Done
assignee: []
created_date: '2026-05-22 00:50'
updated_date: '2026-05-22 01:11'
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
- [x] #1 nucleus/backends/pthreads-sync/tests/multi_worker_check_frame.rs pins static decl + guard local + per-thread Log/Count branch emit-strings for a synthetic 2-worker fixture.
- [x] #2 nucleus/backends/mp-tcp-bufsync/tests/multi_worker_check_frame.rs does the same for the multi-process backend.
- [x] #3 A regression that inlines a writeln! back into either multi_worker site (breaking the shared-helper invariant) trips the new tests.
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 23 (2026-05-22): 4 multi-worker emit-string pinning tests added. Closes the cycle-22 review-gate B.1 gap: the shared template helpers (TASK-0222) were byte-transparent on multi-worker emit paths only by shared-helper CONSTRUCTION; now they are byte-transparent by TEST too.

pthreads-sync (extended existing tests/multi_worker.rs):
- multi_worker_check_loop_log_emit_pins_per_thread_eprintln_template: partition=workers (2 compute workers); asserts exactly 2 eprintln Log template sites in main.rs (one per spawned worker).
- multi_worker_check_loop_count_emit_pins_static_guard_and_fetch_add_templates: same fixture with on_violation=count; asserts 1 file-scope static (deduped by ident across workers per TASK-0052.05) + 1 Drop guard local (host thread owns) + 2 fetch_add sites (one per worker) + 1 struct NucCheckCountReporter declaration. Cross-checks no leakage of Panic/Log templates.

mp-tcp-bufsync (extended tests/check_frame_emit.rs):
- mp_tcp_bufsync_multi_worker_log_emit_pins_per_thread_eprintln_template: 3 worker bins (host + w0 + w1); asserts exactly 2 eprintln sites total across all per-worker bins (host has no check_frame, partition=workers projects onto compute workers only).
- mp_tcp_bufsync_multi_worker_count_emit_pins_static_guard_and_fetch_add: each compute worker is a SEPARATE PROCESS so each has its OWN static + guard + struct + fetch_add (unlike pthreads-sync's shared-static-across-threads). Asserts 2 of each across the 2 compute workers' bins.

A regression that inlines a writeln back into a multi-worker site (breaking the shared-helper invariant) now trips one or more of these tests.

Gate: cargo test --workspace 575 / 0 / 2 (was 571; +4 new pins). Clippy clean. just e2e 36/29/0/7 preserved.

AC#3 verified: the synthetic 2-worker fixture (CHECK_ALGO_SRC + variants of CHECK_SCHED_SRC with on_violation=log|count) IS the same shape Wave B-2 will use for pthreads-async's equivalent test file (TASK-0228 AC#5).
<!-- SECTION:FINAL_SUMMARY:END -->
