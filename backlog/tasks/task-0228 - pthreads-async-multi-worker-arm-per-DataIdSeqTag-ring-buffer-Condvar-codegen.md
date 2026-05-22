---
id: TASK-0228
title: >-
  pthreads-async multi-worker arm + per-(DataId,SeqTag) ring buffer + Condvar
  codegen
status: To Do
assignee: []
created_date: '2026-05-21 21:49'
updated_date: '2026-05-22 00:50'
labels:
  - M4
  - backend
  - multi-worker
dependencies:
  - TASK-0226
  - TASK-0222
  - TASK-0233
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

## Cycle 18 review-gate corrections (HIGH D.1 wording)

Cycle 18's prior notes claimed 'Wave A delivers AC#1 fully'. The architect review-gate correctly noted this OVERCLAIMS: AC#1 literal text says 'File-scope Ring<T> struct is EMITTED ONCE PER FILE with the documented push/wait semantics'. The phrase 'emitted once per file' implies the helper is CALLED from emit() and produces output in the generated file. Wave A delivers only the helper (pure function returning string); the multi-worker arm still ContractGaps, so no Ring<T> is emitted by any actual codegen path.

CORRECTION: Wave A delivers AC#1 PARTIALLY — the SHAPE half ('with the documented push/wait semantics, Mutex<VecDeque<T>> + Condvar pair, capacity-in-instance') is met by the helper. The EMISSION half ('emitted once per file') closes ONLY in Wave B, when the helper is called from emit() during the multi-worker codegen path. AC#1 will not be ticked until Wave B integrates the helper.

Similarly AC#8 is PARTIALLY met: the Ring<T> struct shape pin lives in tests/multi_worker_codegen.rs (closed); the 'representative push/wait pair' pin requires actual push/wait emit code which only exists after Wave B.

## Cycle 18 review-gate medium fixes applied (cycle 18 in-thread lockstep)

* tests/multi_worker_codegen.rs: added file-level docstring note explaining the aspirational filename + Wave B coverage.
* tests/multi_worker_codegen.rs: added ring_struct_decl_negative_checks_pin_design_decisions test — pins !notify_all, !if-vs-while spurious-wakeup safety, !with_capacity(0) defensive negative checks.
* tests/multi_worker_codegen.rs: added ring_struct_decl_has_exactly_four_fields test — pins struct field count (catches future cycle silently adding a 5th field).
* src/ring_buffer.rs: tightened emit_ring_instance_decl docstring from 'cap >= D' to 'cap >= max(1, D)', citing both upstream gates (ZeroBufferOption + PipelineExceedsBuffer).
* TASK-0232 filed (LOW): cross-backend lockstep harden Mutex::lock() unwrap -> expect across pthreads-sync Slot<T> + pthreads-async Ring<T>. Deferred (cross-backend coordination; not Wave A's scope).

## Cycle 19: TASK-0233 precondition landed (2026-05-22)

Wave B needs per-(DataId, SeqTag) ring sizing from the schedule's 'transfer DATA : buffer=N' directive. The value lives in ACFG::XferPlaceholder::policy.buffer but the backend receives only NameSidecar per the EventList contract (TASK-0124). Cycle 19 (commit pending) closes this gap as TASK-0233:

- NameSidecar.transfer_buffer_for_seq: BTreeMap<SeqTag, u64> — new field with serde-default.
- build_sidecar populates it by walking ACFGNode tree (Operation/Sync no-op; Xfer extract seq+buffer; Repeat/Sequence recurse).
- 4 unit tests pin the invariant: async pipeline_parallel populates with mix of 1 + 3 values; sync naive is empty; multi-worker sync (02-split) is non-empty with all-1; the walker descends Repeat (defensive cross-check vs independent ACFG walk).

TASK-0228 now depends on TASK-0233. Wave B can consume the new field directly:

  let cap = sidecar.transfer_buffer_for_seq[&seq];
  emit_ring_instance_decl(&mut out, &format!('ring_{ring_id}'), &rty, cap);

— no ACFG access, no Event variant change. EventList contract preserved.

## Cycle 20 (2026-05-22) — Wave B-1 landed: Plan data structure

Pure data-structure cycle. The Plan struct captures every fact a Wave B-2 emit() will need:
- used_workers (filter per_worker by non-empty events; matches pthreads-sync host election)
- host_worker (named 'host' else smallest WorkerId)
- ring_ids: BTreeMap<(DataId, SeqTag), RingId> — assigned ascending 0..N-1 deterministically
- ring_caps: BTreeMap<(DataId, SeqTag), u64> — joined from sidecar.transfer_buffer_for_seq (TASK-0233 precondition). Missing entry -> ContractGap with forward-link.
- pair_tiles: BTreeMap<(DataId, SeqTag), IterTile> — for Wave B-2 fan-out gather.
- worker_rings(w): subset of ring_ids that worker w touches via Push/Wait.

Files:
- nucleus/backends/pthreads-async/src/multi_worker.rs: NEW (~280 LoC incl. 200 LoC of in-module unit tests).
- nucleus/backends/pthreads-async/src/lib.rs: added 'mod multi_worker' (pub(crate)).

5 unit tests pin the build invariants:
1. build_rejects_single_worker_with_contract_gap (single-worker arm should not call this)
2. build_succeeds_for_02_split_with_default_buffer_1 (host election by name + all ring_caps == 1)
3. build_succeeds_for_13_pipeline_parallel_with_mixed_buffers (4 used workers + exactly 3 ring_caps == 3 + ring_ids ascending 0..N-1)
4. worker_rings_returns_subset_per_worker (per-worker touch sets are subsets; union covers all rings)
5. build_fails_on_missing_sidecar_buffer_entry (TASK-0233 contract-gap path: empty sidecar.transfer_buffer_for_seq with real per_worker -> fail-loud)

Gate:
- cargo test --workspace: 569 / 0 / 2 (was 564; +5 new Plan tests).
- cargo clippy --workspace --all-targets -- -D warnings: clean (one explicit-counter-loop fix in the test).
- just e2e: 36 / 29 / 0 / 7 baseline preserved.

Wave B-2 scope (next cycle or fresh session): replace emit()'s ContractGap with a Plan::build + render_main_rs_multi call sequence. Emit:
- File-scope Ring<T> struct via ring_buffer::emit_ring_struct_decl().
- One Arc<Ring<T>> per ring_ids entry, sized by ring_caps[key], via ring_buffer::emit_ring_instance_decl().
- Per worker: a thread::spawn closure with the worker's EventList rendered. Event::Push -> ring_<id>.push(value); Event::Wait -> let v = ring_<id>.wait();. Reuse the SHARED pthreads_sync renderers for everything that's not Push/Wait (Fire/Loop/etc.).
- Host thread: input loading + handle joins + output writing.
- Same-worker carveout via TASK-0214 (transfer_inject's src==dst skip means no Push/Wait reaches us for same-worker data).
- Multi-worker check_frame via TASK-0227 (now MULTI-worker scoped, depends on TASK-0222 emit-string extraction).

## Wave deliverables map (cycle-20 review-gate D.1)

To make wave progress legible — AC structure stays as filed; each AC's wave assignment + status:

| AC# | Wave | Status | Notes |
|-----|------|--------|-------|
| #1 (Ring<T> struct emitted once per file) | Wave A helper + Wave B-2 emission | Wave A DONE (helper landed cycle 18); EMISSION pending Wave B-2 | Two halves; shape complete, integration pending |
| #2 (Per-(DataId,SeqTag) Arc<Ring<T>> sized N=buffer, starts EMPTY) | Wave A helper + Wave B-1 Plan + Wave B-2 emission | Wave A helper DONE; Wave B-1 Plan ring_caps populated DONE; EMISSION pending Wave B-2 | Three halves; same as #1 |
| #3 (Same-worker carveout) | Wave B-2 (verify-by-construction via TASK-0214 upstream) | Pending | Should be free — transfer_inject already skips src==dst |
| #4 (Per-worker thread::spawn + Push/Wait dispatch) | Wave B-2 | Pending | The bulk of B-2 work |
| #5 (Multi-worker check_frame) | Wave B-2 — depends on TASK-0222 (template extraction) | Pending | Will not land until TASK-0222 lands |
| #6 (Per-fan-out-pair sizing) | Wave B-1 Plan ring_ids structurally + Wave B-2 emission | Wave B-1 partial DONE (key is (DataId, SeqTag), fan-out tie-breaks correctly); EMISSION pending B-2 | No fan-out test yet (cycle-20 review-gate B.2) |
| #7 (Workspace gates preserved) | Every wave gate-tracked | DONE for each cycle |
| #8 (Codegen-string assertion tests pin Ring<T> + push/wait pair) | Wave A (struct) + Wave B-2 (push/wait pair) | Wave A struct shape DONE (4 negative+positive tests); push/wait pair tests pending B-2 |

## Wave B-2 entry-criteria (decide BEFORE writing render_main_rs_multi)

- **TASK-0234** (cycle 20 filed): decide Event::Sync handling — ContractGap-reject vs Barrier emit. Wave B-1's Plan SILENTLY SKIPS Event::Sync; Wave B-2 must close this gap explicitly.
- **TASK-0222** (still To Do): template extraction is a precondition for TASK-0228 AC#5 (multi-worker check_frame). Wave B-2's check_frame work BLOCKS on TASK-0222 landing.
- **Realistic estimate**: Wave B-2 is ~2-4 cycles in itself (~800 LoC of render_main_rs_multi mirroring pthreads-sync's, plus per-worker dispatch, plus compile-check integration test). The cycle-20 close 'next cycle or fresh session' phrasing was optimistic; revise the next implementer's expectation downward.

## Cycle-20 review-gate lockstep fixes applied in-thread (this commit + amend)

- LOW E.2 (.expect → .ok_or_else): aligned with pthreads-sync precedent. Now typed ContractGap if the upstream guard regresses.
- LOW A.5 (debug_assert ring_ids.len() == ring_caps.len()): added at Plan::build site so a production-build regression catches the join collapse (the test pins it already; the assert is defense-in-depth).
- A.1 / E.3 (Event::Sync silently skipped): now docstring-flagged on collect_xfer_pairs with forward-link to TASK-0234.
- D.1 (AC list doesn't reflect wave progress): this notes block IS the Wave deliverables map (architect recommended option (b)).

## Cycle-20 review-gate items NOT applied this cycle

- B.2 (fan-out test fixture using 05-stencil/distributed): the schedule's  exceeds the sync tier-1 capability surface but should still LOWER cleanly for sidecar testing. Adding the test is bounded but I'm leaving it for cycle 21 — deferring rather than over-running this cycle's budget.
- C.1 (per-item dead_code allows): when Wave B-2 removes the module-level allow, audit each unused item then.
- C.2 (pair_tiles consumer hypothetical): Wave B-2 must consume or delete. Documented as Wave B-2 must-decide.
- A.2 (guard message conflates n==0/n==1): nice-to-have polish in Wave B-2 commit.

## Cycle 21 (2026-05-22) — TASK-0234 closed Done: barrier_participants in Plan

The Plan struct now carries barrier_participants: BTreeMap[SyncTag, BTreeSet[WorkerId]] populated by walking Event::Sync. Mirrors pthreads-sync's multi_worker::Plan field-for-field. Wave B-2 emits std::sync::Barrier from this shape exactly like pthreads-sync.

Two new tests: build_populates_barrier_participants_for_multi_worker_sync_schedule + build_records_one_entry_per_unique_sync_tag.

Wave B-2's two preconditions: TASK-0234 (Event::Sync) Done; TASK-0222 (template extraction for AC#5) still To Do.

Updated Wave deliverables map: AC#4 (per-worker thread::spawn) now has barrier_participants ready for emit; the remaining Wave B-2 work for that AC is the actual code emission. AC#5 (multi-worker check_frame) still blocks on TASK-0222.

## Cycle 22 (2026-05-22) — TASK-0222 helpers landed (AC#1/2)

The 4 emit-string templates are extracted into pthreads-sync as pub helpers (emit_count_static, emit_count_guard_local, emit_log_branch, emit_count_branch). Two backends (pthreads-sync + mp-tcp-bufsync) consume them; pthreads-async will consume in Wave B-2.

TASK-0228 AC#5 unblocked: the multi-worker check_frame work can now reuse the same 4 templates that pthreads-sync's multi_worker.rs uses (it already migrated to the helpers in cycle 22).

Wave B-2 status: zero remaining preconditions in the tracker.
- TASK-0234 (Event::Sync) — Done (cycle 21).
- TASK-0222 (template extraction) — AC#1/2 done (cycle 22); AC#3 closes with TASK-0228 AC#5 (a new pthreads-async/tests/check_frame_emit.rs test file).

Wave B-2 is now FULLY UNBLOCKED. Estimated 2-4 cycles remaining (mirror pthreads-sync's render_main_rs_multi + per-worker thread::spawn dispatch + compile-check integration test).

## Cycle 22 review-gate corrections (HIGH D.2 + B.1)

HIGH D.2 — TASK-0226 tracker-vs-source drift fixed in lockstep:
TASK-0226 (single-worker straight-line emit) was implemented in source by cycle 17 (commit 5719897) but its tracker status was never updated. Cycle-22 architect review caught this. TASK-0226 NOW closed Done with 5 of 6 ACs ticked + Final Summary (AC#3 left partial because the literal 'DELETE' instruction was substituted by repurposing the test for multi-worker, which met the intent but not the literal text).

The cycle-22 commit message's claim 'Wave B-2 is now FULLY UNBLOCKED' is therefore now genuinely true (the tracker says so), where previously it relied on source-state.

Wave B-2 preconditions corrected status:
- TASK-0226 (single-worker arm) — Done (in source since cycle 17; tracker closed cycle 22).
- TASK-0233 (sidecar buffer-for-seq) — Done (cycle 19).
- TASK-0234 (Event::Sync handling) — Done (cycle 21).
- TASK-0222 (template extraction) — AC#1/2 Done (cycle 22); AC#3 closes WITH TASK-0228 AC#5.

HIGH B.1 — test-coverage gap for multi-worker emit paths:
The shared helpers' byte-transparency is proven by 17/17 single-worker pinning tests. The multi-worker emit paths in pthreads-sync's multi_worker.rs + mp-tcp-bufsync's render_worker_program are byte-transparent ONLY by shared-helper construction (one helper → multiple callers cannot drift relative to each other). This is real protection against template-text drift; it is NOT protection against the call-graph drifting (someone inlining a writeln back). Filed as TASK-0236.

When Wave B-2 lands, the same multi-worker pinning shape should also cover pthreads-async, addressing TASK-0222 AC#3.c by construction.
<!-- SECTION:NOTES:END -->
