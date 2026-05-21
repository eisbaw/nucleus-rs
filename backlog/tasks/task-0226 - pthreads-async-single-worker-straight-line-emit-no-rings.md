---
id: TASK-0226
title: pthreads-async single-worker straight-line emit (no rings)
status: To Do
assignee: []
created_date: '2026-05-21 21:49'
updated_date: '2026-05-21 22:09'
labels:
  - M4
  - backend
dependencies:
  - TASK-0042.01
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0042.01 cycle 16 (2026-05-21) left a SKELETON: pthreads_async::emit returns EmitError::ContractGap. This task lands the SINGLE-WORKER (zero or one used worker) codegen body — the straight-line emit that mirrors pthreads-sync's single-worker emitter, with NO ring buffers (rings are multi-worker only — see corrected TASK-0228).

Scope: when used_workers.len() <= 1, pthreads_async::emit reuses pthreads_sync::render_single_worker_main exactly like mp-tcp-bufsync does today (see backends/mp-tcp-bufsync/src/lib.rs:158-180). The emitted arithmetic is byte-identical to pthreads-sync's single-worker output by construction. For used_workers.len() >= 2, continue to return EmitError::ContractGap pointing at TASK-0228 (which is the multi-worker + ring-buffer headline work).

Rationale: 'pthreads-async single-worker straight-line emit' is structurally identical to 'pthreads-sync single-worker straight-line emit' because no cross-worker transfer exists when there is one worker — Push/Wait would be ContractGaps in pthreads-sync today (lib.rs:1019). By reusing the shared single-worker renderer, this backend's arithmetic stays byte-identical to pthreads-sync for any naive schedule, which is the cross-backend differential invariant.

Why split from TASK-0228: the single-worker emit is small, mechanical, and lets the 'capability surface satisfiable + codegen runnable' assertion hold for the broadest possible schedule set immediately. Multi-worker + rings is the multi-cycle headline. This task is single-cycle.

Read FIRST: backends/mp-tcp-bufsync/src/lib.rs:158-180 (the 'single process: reuse the SHARED single-worker renderer' precedent). Then the skeleton at nucleus/backends/pthreads-async/src/lib.rs — the ContractGap early-return is replaced by a branch on used_workers.len().
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 render_array_init_for / render_const_expr_pub / render_fire_args_pub / render_single_worker_main / rust_type_of are reused from pthreads_sync::* — no expr/index/call renderer is duplicated (drift control).
- [ ] #2 Workspace tests pass, clippy -D warnings clean, just e2e baseline preserved (this task does NOT add e2e cells — those are TASK-0229).
- [ ] #3 Of the two skeleton smoke tests in nucleus/backends/pthreads-async/tests/skeleton.rs: (a) DELETE skeleton_emit_returns_contract_gap_with_task_0226_forward_link (the ContractGap message is gone once codegen lands). (b) KEEP emit_result_shape_is_single_binary_five_fields (the EmitResult struct still exists; the compile-time shape pin still protects the driver dispatch arm from silent struct drift). Move or rename if desired but do not delete.
- [ ] #4 pthreads-async emit() detects used_workers.len() <= 1 and emits a runnable Cargo.toml + src/main.rs + src/kernels.rs + run.sh Cargo project by delegating to the SHARED pthreads_sync::render_single_worker_main.
- [ ] #5 Emitted arithmetic is byte-identical to pthreads-sync for any naive schedule (verifiable by a unit test that builds the same EventList through both backends and string-compares the per-iteration kernel call site).
- [ ] #6 used_workers.len() >= 2 continues to return EmitError::ContractGap pointing at TASK-0228 (the multi-worker + ring-buffer task), with a precise forward-link.
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Review-gate finding (TASK-0042.01 cycle 16 review)

MEDIUM: as originally filed AC#4 said 'the skeleton smoke test is REMOVED' — but the skeleton.rs file has TWO tests, only one of which becomes obsolete when codegen lands. The other (emit_result_shape_is_single_binary_five_fields) is a compile-time tripwire on the EmitResult struct that the driver dispatch arm reads — deleting it would silently allow driver/struct drift.

Fixed in-thread by splitting AC#4: delete the ContractGap-message test, keep (or migrate) the struct-shape pin.

## Tracker correction (cycle 17, 2026-05-22)

Cycle 16 filed TASK-0226 with title 'single-worker ring-buffer + Condvar codegen' but the scope (per (DataId, SeqTag) ring + Push/Wait emit) is intrinsically MULTI-WORKER — single-worker has no cross-worker Push/Wait so the ring buffer never fires. The architect review (cycle 16) did not catch this contradiction; self-review in cycle 17 surfaced it.

Fix (this edit): re-scoped to JUST single-worker straight-line emit (mirrors pthreads-sync; reuses shared helpers). Ring buffer + multi-worker codegen consolidated into corrected TASK-0228 (separate edit). ACs realigned. The post-TASK-0213 ring-EMPTY contract + buffer=N sizing — those forward-carries now belong to TASK-0228, not here.
<!-- SECTION:NOTES:END -->
