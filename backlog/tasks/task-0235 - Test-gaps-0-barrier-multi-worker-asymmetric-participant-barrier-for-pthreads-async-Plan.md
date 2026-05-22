---
id: TASK-0235
title: >-
  Test gaps: 0-barrier multi-worker + asymmetric-participant barrier for
  pthreads-async Plan
status: Done
assignee:
  - mped-architect-impl
created_date: '2026-05-22 00:14'
updated_date: '2026-05-22 10:10'
labels:
  - M4
  - backend
  - test-coverage
dependencies:
  - TASK-0234
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-21 review-gate C.2 / F.3 finding (commit 7bacc17): the two new TASK-0234 tests pin barrier_participants correctness for the COMMON case (non-empty + uniform participants per tag) but miss two edge scenarios:

(a) 0-barrier multi-worker schedule. The Plan's barrier_participants should be EMPTY when no Event::Sync exists. A future walker regression that synthesised a phantom tag would pass the existing tests. No e2e fixture has zero barriers (every multi-worker schedule produces at least one inject_syncs barrier), so this needs a SYNTHETIC per_worker test fixture (mirror skeleton.rs's two_workers_empty helper but with one Event::Push + one Event::Wait + zero Event::Sync).

(b) Asymmetric-participant barriers. pthreads-sync's partial_nonuniform_barrier_multi_worker_lowers_correctly fixture covers this for the sync backend. An analogous test for pthreads-async's Plan would assert: barrier_participants[tag1].len() != barrier_participants[tag2].len() when the schedule has non-uniform barriers. Confirms the docstring claim 'partial / non-uniform barriers lower correctly'.

These are deferrable: Wave B-2's emit-string tests will naturally exercise (a) (empty-map iteration), and pthreads-sync's existing fixture demonstrates (b) works at the sync level. Filing as a separate follow-up to keep TASK-0234 focused and avoid scope creep.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Test fixture (a): synthetic 2-worker per_worker with Push/Wait but no Sync; asserts barrier_participants is empty.
- [x] #2 Test fixture (b): asymmetric-participant barriers (likely synthetic, mirror pthreads-sync's partial_nonuniform fixture); asserts barrier_participants entries have differing parts.len() across tags.
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
PLAN:
1. Add two synthetic tests in pthreads-async multi_worker.rs mod tests block.
2. Test (a) build_zero_barrier_multi_worker_has_empty_barrier_participants:
   - Construct 2-worker per_worker (WorkerId(0), WorkerId(1)).
   - Worker 0: Event::Push { dst=W1, data=D0, tile=empty, seq=S0 }.
   - Worker 1: Event::Wait { src=W0, data=D0, tile=empty, seq=S0 }.
   - sidecar: NameSidecar::default() with transfer_buffer_for_seq[S0]=1.
   - names: NameTables::default().
   - Assert plan.barrier_participants.is_empty().
3. Test (b) build_asymmetric_participant_barriers_lower_correctly:
   - Construct 3-worker per_worker (host=W0, w0=W1, w1=W2).
   - Two SyncTag values: A=SyncTag(0), B=SyncTag(1).
   - host events: [Sync{A, {host,w0}}, Sync{B, {host,w0,w1}}].
   - w0 events: [Sync{A, {host,w0}}, Sync{B, {host,w0,w1}}].
   - w1 events: [Sync{B, {host,w0,w1}}] (NO tag A).
   - No Push/Wait.
   - sidecar: NameSidecar::default() (empty transfer_buffer_for_seq).
   - names: NameTables::default().
   - Assert plan.barrier_participants[A].len() == 2.
   - Assert plan.barrier_participants[B].len() == 3.
   - Assert sizes differ.
4. Run cargo test -p pthreads-async --lib multi_worker::tests.
5. Run just test, just clippy, just e2e.
6. 5x stress on cargo test -p pthreads-async.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Implementation notes (cycle-33)

Two synthetic tests added to `nucleus/backends/pthreads-async/src/multi_worker.rs` in the `mod tests` block (after `build_fails_on_missing_sidecar_buffer_entry`):

- **AC#1** `build_zero_barrier_multi_worker_has_empty_barrier_participants` — `multi_worker.rs:889-947`.
  - 2-worker per_worker: host carries one Event::Push, w_b carries the matching Event::Wait; ZERO Event::Sync.
  - sidecar.transfer_buffer_for_seq[SeqTag(0)] = 1 (required so Plan::build's TASK-0233 cap lookup does not gap).
  - Asserts: used_workers.len() == 2; ring_ids.len() == 1; barrier_participants.is_empty().

- **AC#2** `build_asymmetric_participant_barriers_lower_correctly` — `multi_worker.rs:949-1043`.
  - 3-worker per_worker (host=W0, w0=W1, w1=W2); two SyncTags with different participant set sizes.
  - tag_A = {host, w0} (size 2); tag_B = {host, w0, w1} (size 3).
  - host events: [Sync(A), Sync(B)]; w0 events: [Sync(A), Sync(B)]; w1 events: [Sync(B)] (asymmetric).
  - No Push/Wait → no transfer_buffer_for_seq lookup → empty sidecar OK.
  - Asserts: used_workers.len() == 3; ring_ids empty; barrier_participants.len() == 2; |A|=2 != |B|=3; exact set identity (catches a union-regression that would canonicalise the participant sets).

## Gate results

- `cargo test -p pthreads-async --lib multi_worker::tests`: **9/9 pass** (2 new + 7 pre-existing).
- `just test`: **0 FAILED** across the workspace.
- `just clippy`: clean (Finished dev profile).
- `just e2e`: total 54, pass 47, fail 0, skipped 7, required-fail 0 — unchanged.
- 5x stress on `cargo test -p pthreads-async`: **0/5 FAILED** — 9/9 every run, deterministic.

## Limits / honest caveats

- Synthetic-only: both tests construct `per_worker` directly via `BTreeMap` literals — they do NOT run `acfg_to_events` and so are decoupled from any upstream projection regression. The TASK-0234 fixture-driven tests still cover the projection path; these are complementary, not a replacement.
- Test (b) carries no Push/Wait, so the `pair_tiles` → `ring_caps` lookup is trivially empty. A future regression where Plan::build mis-handles the BOTH-rings-AND-barriers case is already covered by `build_succeeds_for_13_pipeline_parallel_with_mixed_buffers` + `build_records_one_entry_per_unique_sync_tag` (which exercise mixed rings+barriers via the real 13-cnn-inference/pipeline_parallel fixture).
- The asymmetric-set identity assertion (`a_parts == &parts_a`) pins the walker's first-sighting `or_insert_with` rule; if a future change unified sets across tags (e.g. union semantics), this would fail loud. That extra assertion goes beyond the minimal AC#2 (.len() difference) and is intentional belt-and-braces.

## Status

Plan::build's actual behavior matches both AC#1 and AC#2 expectations — no follow-up HIGH bug filed.

READY FOR REVIEW + COMMIT.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 33 (2026-05-22) — closed. Added 2 synthetic Plan-level tests in nucleus/backends/pthreads-async/src/multi_worker.rs (mod tests block, ~+173 lines):

- AC#1 build_zero_barrier_multi_worker_has_empty_barrier_participants: 2-worker fixture with one cross-worker Push/Wait pair (so used_workers.len()==2 and ring_ids.len()==1 — non-degenerate Plan) AND zero Event::Sync. Asserts barrier_participants.is_empty(). Regression catch: a future walker bug that synthesises a phantom SyncTag fails this test.

- AC#2 build_asymmetric_participant_barriers_lower_correctly: 3-worker fixture with two SyncTags — barrier_A participants {host, w0} (size 2), barrier_B participants {host, w0, w1} (size 3). Asserts |A|=2, |B|=3, |A|!=|B|, plus exact set identity. Captures the spirit of pthreads-sync's partial_nonuniform_barrier_multi_worker_lowers_correctly (cycle-21 reference) at the Plan level.

Honest limit (architect-flagged MEDIUM, deferred): the async tests are Plan-level only, not codegen/runtime. They prove the walker produces the right shape but not that the emitted async code actually waits on the right barrier participants. The sync analogue goes through full e2e; this test is weaker reach but same invariant.

Gate: 9/9 in-module Plan tests pass; just test 0 FAILED; just clippy clean; just e2e 54/47/0/7 unchanged; 5x stress 0/5 FAILED (QA reviewer caught and corrected an implementer-side stress-loop cwd bug; the underlying stability is genuine).

Review-gate: both qa-test-runner + mped-architect GO. No HIGH/MEDIUM blocking. LOW honest-limit note about Plan-only reach captured here; codegen-level coverage already provided via the pipeline_parallel real-fixture test added cycle 21.
<!-- SECTION:FINAL_SUMMARY:END -->
