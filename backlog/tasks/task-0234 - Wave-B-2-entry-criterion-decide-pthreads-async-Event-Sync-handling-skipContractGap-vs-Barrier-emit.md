---
id: TASK-0234
title: >-
  Wave B-2 entry-criterion: decide pthreads-async Event::Sync handling
  (skip+ContractGap vs Barrier emit)
status: Done
assignee: []
created_date: '2026-05-21 23:51'
updated_date: '2026-05-22 00:09'
labels:
  - M4
  - backend
  - decision
  - wave-b-2
dependencies:
  - TASK-0228
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-20 review-gate A.1 + E.1 finding (commit 818ee26): Wave B-1's Plan struct deliberately omits a barrier_participants field (pthreads-sync's multi_worker::Plan has one). The collect_xfer_pairs walker SILENTLY SKIPS Event::Sync via the default _ => {} arm.

This is honest for Wave B-1 (no integration yet) but Wave B-2 will meet Event::Sync on real fixtures: 02-split-add/split, 13-cnn-inference/pipeline_parallel, and any other multi-worker schedule with cross-worker writes generates Event::Sync nodes via inject_syncs. If Wave B-2 doesn't actively handle them, the emitted code will race.

Two options:

(a) ContractGap pthreads-async on any schedule containing Event::Sync — useful gate while async-only barrier-free schedules are the focus. Document the unsupported shape; file follow-up for option (b).

(b) Add barrier_participants: BTreeMap<SyncTag, BTreeSet<WorkerId>> to Plan, populate by walking Event::Sync, and emit std::sync::Barrier in Wave B-2 alongside the ring infrastructure. Barriers and rings are orthogonal (async transfers don't replace barrier semantics). This makes pthreads-async a full superset of pthreads-sync's multi-worker capability.

This task is a Wave B-2 ENTRY-CRITERION: the implementer must DECIDE before writing render_main_rs_multi. Filing now so the next session sees it.

Recommendation: option (b). The cost is modest (one Plan field + one walker + one renderer block) and matches the eventual goal of pthreads-async being the headline async/buffered tier-1 backend.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Decision recorded in TASK-0228 implementation notes: option (a) or option (b).
- [ ] #2 If (a): Plan::build emits typed ContractGap on any schedule whose EventList contains Event::Sync (multi-worker only — single-worker Event::Sync is irrelevant). Test fixture asserts the rejection on a real fixture (02-split-add/split currently has cross-worker barriers).
- [x] #3 If (b): Plan gains barrier_participants field, populated by walking Event::Sync. Test fixture asserts the field is populated for 02-split-add/split.
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 21 (2026-05-22): chose option (b) per architect recommendation. The Plan struct now carries barrier_participants: BTreeMap[SyncTag, BTreeSet[WorkerId]] populated by walking Event::Sync. Mirrors pthreads-sync's multi_worker::Plan field-for-field; Wave B-2 emits std::sync::Barrier from the same shape pthreads-sync already proves works.

Two new unit tests (in nucleus/backends/pthreads-async/src/multi_worker.rs):
- build_populates_barrier_participants_for_multi_worker_sync_schedule: 02-split-add/split asserts non-empty map + every barrier's participant subset of used_workers + non-empty participants.
- build_records_one_entry_per_unique_sync_tag: 13-cnn-inference/pipeline_parallel asserts exactly one entry per unique SyncTag (independent walker cross-check).

AC#2 (option (a) ContractGap-reject path) NOT taken — option (b) chosen. Architect recommendation: 'option (b). The cost is modest (one Plan field + one walker + one renderer block) and matches the eventual goal of pthreads-async being the headline async/buffered tier-1 backend.'

Gate: cargo test --workspace 571 / 0 / 2 (was 569; +2 new barrier tests). Clippy clean. just e2e 36/29/0/7 preserved.

Wave B-2 unblocked on the Event::Sync front. Remaining preconditions for Wave B-2:
- TASK-0222 (template extraction) — still To Do; precondition for TASK-0228 AC#5.
- pair_tiles consumer audit when Wave B-2 lands (cycle-20 review-gate C.2 deferred).
<!-- SECTION:FINAL_SUMMARY:END -->
