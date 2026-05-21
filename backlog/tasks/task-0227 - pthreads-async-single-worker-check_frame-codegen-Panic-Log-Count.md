---
id: TASK-0227
title: pthreads-async single-worker check_frame codegen (Panic/Log/Count)
status: To Do
assignee: []
created_date: '2026-05-21 21:49'
labels:
  - M4
  - backend
  - check-frame
dependencies:
  - TASK-0226
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
After TASK-0226 lands the ring-buffer + Condvar codegen, this task wires check_frame onto the third tier-1 backend's single-worker arm.

Read the TASK-0052.04 + TASK-0052.05 forward-carry on TASK-0042.01: the shared helpers (sanitize_loop_var, collect_count_check_frames, emit_count_reporter_struct, CountCheckLoop) are pub on pthreads_sync. mp-tcp-bufsync already imports them — pthreads-async should follow the same shape.

Dispatch on frame.on_violation:
- Panic -> panic!("latency budget violated on check loop V: iteration took {} ns, max {ns} ns", _check_elapsed)
- Log   -> eprintln!("warning: check loop V violated latency_max={ns} ns: iteration took {} ns", _check_elapsed)
- Count -> NUC_CHECK_COUNT_<sanitized>.fetch_add(1, Relaxed)
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Event::Loop arm in pthreads-async render_main_rs (or equivalent) dispatches on ViolationKind for Panic / Log / Count, mirroring pthreads-sync + mp-tcp-bufsync.
- [ ] #2 Helpers from pthreads-sync are reused verbatim — no template-string is re-emitted locally (TASK-0222 cleanup precedent: the four code-clone templates can be extracted in a follow-up if it becomes load-bearing).
- [ ] #3 New emit-string test file at nucleus/backends/pthreads-async/tests/check_frame_emit.rs mirrors backends/mp-tcp-bufsync/tests/check_frame_emit.rs and backends/pthreads-sync/tests/check_frame_codegen.rs.
- [ ] #4 Workspace tests pass, clippy -D warnings clean.
<!-- AC:END -->
