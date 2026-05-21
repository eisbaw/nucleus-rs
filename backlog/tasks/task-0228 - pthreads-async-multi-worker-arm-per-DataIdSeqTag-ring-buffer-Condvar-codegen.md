---
id: TASK-0228
title: >-
  pthreads-async multi-worker arm + per-(DataId,SeqTag) ring buffer + Condvar
  codegen
status: To Do
assignee: []
created_date: '2026-05-21 21:49'
updated_date: '2026-05-21 22:10'
labels:
  - M4
  - backend
  - multi-worker
dependencies:
  - TASK-0226
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
After TASK-0226 lands single-worker, the multi-worker path needs a defined behaviour even before its full implementation. Mirror the pattern pthreads-sync established (multi_worker.rs Plan::emit): if used_workers.len() >= 2 the arm runs.

INITIAL behaviour (this task): reject with EmitError::ContractGap('pthreads-async: multi-worker pipelined arm not yet implemented (see TASK-0228.01)'). This makes the single-worker arm shippable + the multi-worker shape decidable + the failure mode HONEST.

FULL implementation deferred to TASK-0228.01 (filed once TASK-0226 + this task land): per-fan-out-pair (DataId, SeqTag) ring sized N=buffer; the same SHARED static + Drop guard pattern from pthreads-sync multi_worker.rs for check_frame; partition=workers + pipeline=D projects per-pair rings — see TASK-0216 forward-carry.

Read the TASK-0052.05 forward-carry on TASK-0042.01 for the multi-worker check_frame contract; the same panic=abort SIGABRT gotcha applies (Cargo.toml profile.release panic="abort" -> worker thread panic -> whole-process SIGABRT not exit-101).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 File-scope Ring<T> struct is emitted once per file with the documented push/wait semantics (Mutex<VecDeque<T>> + not_empty/not_full Condvars, capacity baked into the instance not the type).
- [ ] #2 Per (DataId, SeqTag) Arc<Ring<T>> instance sized N=buffer (the transfer's buffer=N directive); ring starts EMPTY (no pre-fill, per post-TASK-0213 contract).
- [ ] #3 Same-worker transfer carveout: producer + consumer on the same worker emit no ring/Push/Wait (mirror transfer_inject's src==dst skip + TASK-0214 link-layer carveout).
- [ ] #4 Per worker, a thread::spawn with that worker's EventList rendered to Rust; Event::Push and Event::Wait dispatch into the ring instance keyed by (DataId, SeqTag).
- [ ] #5 Multi-worker check_frame: file-scope shared static AtomicU64 deduped by sanitized ident; Drop guard on host thread (TASK-0052.05 forward-carry). The shared helpers from pthreads-sync (sanitize_loop_var, collect_count_check_frames, emit_count_reporter_struct, CountCheckLoop) ARE used after TASK-0222 extracts the four emit-string templates into shared form.
- [ ] #6 Per-fan-out-pair sizing (TASK-0216 forward-carry): if a data symbol fans out to multiple workers, one ring per producer-consumer pair (each sized N).
- [ ] #7 Workspace tests pass, clippy -D warnings clean, just e2e baseline preserved (e2e cells land in TASK-0229 separately).
- [ ] #8 Codegen-string assertion tests at nucleus/backends/pthreads-async/tests/multi_worker_codegen.rs pin the Ring<T> struct shape + a representative push/wait pair.
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Tracker correction (cycle 17, 2026-05-22)

Cycle 16 filed TASK-0228 as 'multi-worker arm initial ContractGap-reject; full impl deferred to TASK-0228.01'. Combined with the misframed TASK-0226 (which said 'single-worker ring-buffer + Condvar codegen' — internally contradictory), the result was three tasks whose scope didn't map cleanly to the codegen reality.

Fix (this edit): TASK-0228 now IS the full multi-worker + ring-buffer headline work. The 'initial ContractGap-reject' subtask vanishes (TASK-0226 already implements that — single-worker emits, multi-worker rejects with forward-link). The original 'TASK-0228.01 filed when this lands' clause is dropped; this task IS the full work.

If the work later proves too large for one cycle, file a SUB-task at THAT point with the precise carved scope — don't pre-decompose now.
<!-- SECTION:NOTES:END -->
