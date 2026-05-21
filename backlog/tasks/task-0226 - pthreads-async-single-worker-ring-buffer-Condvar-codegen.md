---
id: TASK-0226
title: pthreads-async single-worker ring-buffer + Condvar codegen
status: To Do
assignee: []
created_date: '2026-05-21 21:49'
updated_date: '2026-05-21 22:01'
labels:
  - M4
  - backend
dependencies:
  - TASK-0042.01
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0042.01 cycle 16 (2026-05-21) left a SKELETON: pthreads_async::emit returns EmitError::ContractGap and the capabilities.toml + Cargo wiring + driver dispatch arm are real. This task IS the actual codegen body for the single-worker case.

Scope: per (DataId, SeqTag) emit std::sync::Mutex<VecDeque<T>> + two Condvars (not_empty + not_full); on Event::Push lock + wait on not_full while ring.len()==N + push + notify not_empty; on Event::Wait lock + wait on not_empty while empty + pop + notify not_full. Ring STARTS EMPTY (the post-TASK-0213 corrected contract — see forward-carry on TASK-0042.01).

Read FIRST: TASK-0042.01 notes (forward-carried context: ring contract post-TASK-0213; D is sizing not fill; buffer = N is the only sizing input the runtime needs; recommended Option (c) from the carry: read N directly, ignore D in codegen). Then the skeleton at nucleus/backends/pthreads-async/src/lib.rs — the early-return on Err(EmitError::ContractGap(...)) is the first line you rip out.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 pthreads-async emit() produces a runnable Cargo.toml + src/main.rs + src/kernels.rs + run.sh Cargo project for the single-worker case.
- [ ] #2 Per (DataId, SeqTag) ring is Mutex<VecDeque<T>> + two Condvars; sized N=buffer; starts EMPTY (no pre-fill); producer blocks on full, consumer blocks on empty.
- [ ] #3 render_array_init_for / render_const_expr_pub / render_fire_args_pub / render_single_worker_main / rust_type_of are reused from pthreads_sync::* — no expr/index/call renderer is duplicated (drift control).
- [ ] #4 Workspace tests pass, clippy -D warnings clean, just e2e baseline preserved (this task does NOT add e2e cells — those are TASK-0229).
- [ ] #5 Of the two skeleton smoke tests in nucleus/backends/pthreads-async/tests/skeleton.rs: (a) DELETE skeleton_emit_returns_contract_gap_with_task_0226_forward_link (the ContractGap message is gone once codegen lands). (b) KEEP emit_result_shape_is_single_binary_five_fields (the EmitResult struct still exists; the compile-time shape pin still protects the driver dispatch arm from silent struct drift). Move or rename if desired but do not delete.
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Review-gate finding (TASK-0042.01 cycle 16 review)

MEDIUM: as originally filed AC#4 said 'the skeleton smoke test is REMOVED' — but the skeleton.rs file has TWO tests, only one of which becomes obsolete when codegen lands. The other (emit_result_shape_is_single_binary_five_fields) is a compile-time tripwire on the EmitResult struct that the driver dispatch arm reads — deleting it would silently allow driver/struct drift.

Fixed in-thread by splitting AC#4: delete the ContractGap-message test, keep (or migrate) the struct-shape pin.
<!-- SECTION:NOTES:END -->
