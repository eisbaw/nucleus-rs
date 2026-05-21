---
id: TASK-0228
title: >-
  pthreads-async multi-worker arm + per-(DataId,SeqTag) ring buffer + Condvar
  codegen
status: To Do
assignee: []
created_date: '2026-05-21 21:49'
updated_date: '2026-05-21 22:40'
labels:
  - M4
  - backend
  - multi-worker
dependencies:
  - TASK-0226
  - TASK-0222
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

## Cycle 18 (2026-05-22) — Wave A landed: Ring<T> emit helpers

This task remains To Do (the multi-worker arm is the headline goal; only the runtime substrate is now ready). Wave A scope:

* nucleus/backends/pthreads-async/src/ring_buffer.rs: NEW. Pure-function emit helpers:
  - emit_ring_struct_decl(out: &mut String) — emits the file-scope Ring<T> struct + impl (Mutex<VecDeque<T>>, cap: usize, Condvar pair, push/wait with while-loop block + notify_one). One definition per file, capacity baked into the instance.
  - emit_ring_instance_decl(out, var_name, element_type, cap) — emits one let var: Arc<Ring<T>> = Arc::new(Ring::new(cap)); per (DataId, SeqTag) pair.
* nucleus/backends/pthreads-async/src/lib.rs: mod + pub use ring_buffer::{emit_ring_struct_decl, emit_ring_instance_decl}.
* nucleus/backends/pthreads-async/tests/multi_worker_codegen.rs: NEW. 4 shape-pin tests:
  - ring_struct_decl_pins_documented_shape — pins every field, push semantics, wait semantics.
  - ring_instance_decl_pins_arc_ring_shape — pins the exact byte string for a representative array instance.
  - ring_instance_decl_handles_scalar_element_type — pins a scalar instance shape.
  - ring_struct_decl_does_not_pre_fill_with_d — NEGATIVE check that the emit does NOT contain 'pipeline_depth' / 'pre_fill' / 'initial_marking' (post-TASK-0213 ring-EMPTY contract).

Wave A delivers AC#1 (file-scope Ring<T> with documented push/wait semantics, Mutex<VecDeque<T>> + Condvar pair, capacity-in-instance) + AC#8 partial (codegen-string assertion tests pin Ring<T> shape; the 'representative push/wait pair' part of AC#8 lands when Wave B emits the actual dispatch code).

Wave B (next cycle or later session): integration. Build a Plan struct mirroring pthreads_sync::multi_worker::Plan (lib.rs:392 onward): collect cross-worker (producer, consumer, DataId, SeqTag) tuples, emit Ring<T> struct ONCE per file, one Arc<Ring<T>> per tuple, per-worker thread::spawn body that calls ring_<id>.push(...) on Event::Push and let v = ring_<id>.wait() on Event::Wait. Wave B also covers AC#3 (same-worker carveout) + AC#4 (thread::spawn dispatch) + AC#6 (per-fan-out-pair sizing). AC#5 (multi-worker check_frame) and AC#2 (Arc<Ring<T>> sized N=buffer at the right call site) close inside Wave B.

Why split Wave A from B: integration is multi-cycle; the substrate is independently testable. A drift in the Ring<T> shape now surfaces against focused unit tests, not buried inside a Wave B integration commit.

Gate (cycle 18):
- cargo test --workspace: 557 / 0 / 2 (+4 ring_buffer tests).
- cargo clippy --workspace --all-targets -- -D warnings: clean.
- just e2e: 36 / 29 / 0 / 7 baseline preserved.
<!-- SECTION:NOTES:END -->
