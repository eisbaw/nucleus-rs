---
id: TASK-0227
title: pthreads-async MULTI-WORKER check_frame codegen (Panic/Log/Count)
status: To Do
assignee: []
created_date: '2026-05-21 21:49'
updated_date: '2026-05-21 22:32'
labels:
  - M4
  - backend
  - check-frame
dependencies:
  - TASK-0226
  - TASK-0222
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Review-gate finding (TASK-0042.01 cycle 16 review)

HIGH-severity inconsistency: as filed AC#2 said 'no template-string is re-emitted locally (TASK-0222 cleanup precedent: can be extracted in a follow-up)'. But TASK-0222 explicitly states its trigger is 'when pthreads-async lands as 3rd tier-1 backend' — i.e. the third backend landing IS the extraction trigger, not 'later if it becomes load-bearing'.

Fix (filed in-thread): TASK-0227 now depends on BOTH TASK-0226 (codegen body lands first) AND TASK-0222 (helpers extracted into shared form). Natural order: TASK-0226 lands ring-buffer codegen using the existing duplicated helpers (it's only two backends at that point), then TASK-0222 extracts the 4 emit-string templates into shared form, then TASK-0227 wires check_frame on pthreads-async using the now-shared helpers — so no 3-way clone is ever introduced.

AC#2 reading remains correct under this ordering: 'reused from pthreads_sync::*; no template re-emit' is exactly what's available after TASK-0222 extracts.

## Review-gate finding (cycle 17 review)

HIGH-severity stale-task: original scope said 'single-worker check_frame codegen' for pthreads-async. After TASK-0226 (cycle 17) landed the single-worker arm as a delegation to pthreads_sync::render_single_worker_main, single-worker check_frame is INHERITED FREE — pthreads_sync's render_main_rs path already emits the Panic/Log/Count instrumentation, and delegation means pthreads-async inherits the identical output.

Fix (lockstep this cycle): retitled to MULTI-WORKER check_frame. The single-worker case is now structurally done. The remaining work is the multi-worker check_frame codegen — file-scope shared static AtomicU64 deduped by sanitized ident; Drop guard on host thread; panic=abort SIGABRT gotcha (forward-carried from TASK-0052.05).

Note: TASK-0228 AC#5 already covers this scope. TASK-0227 should be considered consolidated into TASK-0228 AC#5 when TASK-0228 lands. Keeping TASK-0227 as a separate task is a contingency: if the multi-worker check_frame work needs its own cycle outside of TASK-0228's wave, it lives here. Otherwise CLOSE TASK-0227 with a forward-link to TASK-0228 AC#5 at the moment TASK-0228 starts.
<!-- SECTION:NOTES:END -->
