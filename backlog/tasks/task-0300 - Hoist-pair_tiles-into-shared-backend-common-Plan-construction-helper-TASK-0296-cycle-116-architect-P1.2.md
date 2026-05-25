---
id: TASK-0300
title: >-
  Hoist pair_tiles into shared backend-common Plan-construction helper
  (TASK-0296 cycle-116 architect P1.2)
status: Done
assignee:
  - '@mark'
created_date: '2026-05-25 01:18'
updated_date: '2026-05-25 07:59'
labels:
  - backend-common
  - mp-tcp-bufsync
  - refactor
  - hardening
  - forward-carried-from-TASK-0296
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background
TASK-0296 cycle 116 added `pair_tiles: BTreeMap<(DataId, SeqTag), IterTile>` to mp-tcp-bufsync `s Plan` struct, populated by re-walking `per_worker.values()` with `backend_common::multi_worker_walker::collect_xfer_pairs`. The same construction shape is already done inside `WalkerCtx` for pthreads-async + mp-tcp-event.

## Risk
State duplication — both the WalkerCtx-using backends AND mp-tcp-bufsync now collect pair_tiles independently. Two future deltas could drift:
1. A mp-tcp-bufsync-only event source bypasses `per_worker.values()` — pair_tiles becomes stale.
2. A sidecar enrichment of IterTile is consumed in the walker but not by mp-tcp-bufsync (or vice versa).

## Acceptance criteria
1. Lift pair_tiles construction into a shared `backend_common` helper (e.g. `multi_worker_walker::collect_pair_tiles(per_worker) -> BTreeMap<(DataId, SeqTag), IterTile>`).
2. mp-tcp-bufsync uses this helper from Plan::build instead of looping itself.
3. WalkerCtx-using backends pass the same construction result rather than building it inline (or document why they cannot).
4. Sibling check: pthreads-sync also uses the shared walker; verify it benefits from (or is consistent with) this helper.

## Honest scope
- LOW priority — the current duplication is correct by inspection. This is hygiene to keep it that way.
- 1 cycle when picked up. Related to the broader TASK-0284 (lift entire mp-tcp-bufsync per-event walker onto shared multi_worker_walker) — could be done first as a stepping stone or absorbed into TASK-0284.

## Forward-carry from TASK-0296 cycle 116 architect P1.2
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Cycle 130 plan (orchestrator-direct, no implementer subagent per [[feedback-spawned-agents-refuse-code-edits]]):

1. Add pub fn collect_pair_tiles in nucleus/backend-common/src/multi_worker_walker.rs adjacent to collect_xfer_pairs. Signature: pub fn collect_pair_tiles<'a, I: IntoIterator<Item=&'a Vec<Event>>>(events_per_worker: I) -> BTreeMap<(DataId, SeqTag), IterTile>. Doc: 'First-sighting wins on the same (DataId, SeqTag); both endpoints carry the same tile by XferPlaceholder construction (TASK-0018).' Walks Event::Loop recursively via collect_xfer_pairs.

2. Migrate 4 backends:
   - nucleus/backends/mp-tcp-bufsync/src/lib.rs:354-357 -> single helper call.
   - nucleus/backends/mp-tcp-event/src/multi_worker.rs:151-154 -> single helper call (was walker::collect_xfer_pairs; switch to walker::collect_pair_tiles).
   - nucleus/backends/pthreads-async/src/multi_worker.rs:202-205 -> same.
   - nucleus/backends/pthreads-sync/src/multi_worker.rs:203-213 -> let pair_tiles = collect_pair_tiles(per_worker.values()); then slot_ids derives from pair_tiles directly (drop the xfer_pairs temp name).

3. Add a small backend-common test (tests/collect_pair_tiles.rs) that exercises: empty input -> empty map; single Push/Wait -> 1 entry; cross-worker conflicting tiles -> first sighting wins; Push nested inside Event::Loop -> still collected.

4. Cheap gate: nix develop --command bash -c 'just build && just clippy && just test && just test-release && just e2e'. Bit-identical baseline 108/92/0/16/0 MUST be preserved (refactor-only, no semantic change).

5. Commit: 'backend-common + 4 backends: TASK-0300 cycle 130 — hoist pair_tiles into shared backend_common::multi_worker_walker::collect_pair_tiles helper (4 inline 2-3-line builds collapsed; deterministic first-sighting-wins semantics preserved; pin test added)'. No AI co-author trailer per project policy.

6. Parallel read-only review gate (qa-test-runner + mped-architect). Fold P1/P2 in-thread; file follow-ups for larger items.

AC mapping:
- AC#1 (helper exists): step 1.
- AC#2 (mp-tcp-bufsync uses it): step 2 (a).
- AC#3 (WalkerCtx backends share the same construction): step 2 (b)(c) — they call the same helper. Both already build their own pair_tiles then pass it into WalkerCtx; cycle 130 keeps that pattern but the BUILD becomes the shared helper, so the construction is one-source-of-truth. WalkerCtx itself is not refactored (it still receives the map as a parameter — same as today).
- AC#4 (pthreads-sync sibling check): step 2 (d) — pthreads-sync also moves to the helper; was already using collect_xfer_pairs in-place under a temp name.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 130 review-hardening fold-back (commit 6e79d0b + this commit)

Mandatory parallel read-only review gate (qa-test-runner + mped-architect, in parallel):

### qa-test-runner: GO
- just build / clippy / test (858/0/3) / test-release (858/0/3 — identical to dev, no debug_assert divergence per [[feedback-qa-gate-misses-release-profile]]) / e2e (108/92/0/16/0 on BOTH consecutive runs, deterministic) / check-textual-replace-on-codegen / check-include-str-coverage — all 8 arms green.
- The 4 new collect_pair_tiles tests all pass (empty_input_yields_empty_map, single_push_wait_pair, first_sighting_wins_on_conflicting_tiles, push_nested_in_loop_is_collected).

### mped-architect: GO (3 P2 + 1 P3 findings)

Folded in-thread on this same cycle (per cycle-126 fold-back discipline):

- **P2 #1** (docstring nit in tests/collect_pair_tiles.rs:3-5 — said '4-line build' but the four backends were 3-5 lines per the actual diff). FIXED in this commit: docstring tightened to '3-5-line build shape' with pthreads-sync's xfer_pairs rebind cited as the reason for the upper bound.
- **P2 #2** (stale absolute-line citation 'multi_worker.rs:154-161' in mp-tcp-event/tests/multi_worker_emit.rs:303 — the cycle-130 hoist shrank the local layout, so the line range no longer points where the doc-comment promises). FIXED in this commit: migrated to symbolic anchor (search for the 'Wait but no matching Push' ContractGap string in multi_worker.rs's Plan::build). Same class as TASK-0312/0313 sweep.
- **P3 #4** (collect_pair_tiles docstring said 'first-sighting is well-defined because both endpoints carry the same tile by TASK-0018', which explains why first-sighting is SAFE but not what 'first' MEANS). FIXED in this commit: docstring extended with the explicit 'first = lowest-WorkerId per BTreeMap::values() key-ascending iteration' grounding that the cycle-130 test 'first_sighting_wins_on_conflicting_tiles' relies on.

Filed as follow-up (honest-scope):

- **P2 #3** (helper signature could be widened from `&'a Vec<Event>` to `AsRef<[Event]>` for symmetry with the underlying collect_xfer_pairs(&[Event]) primitive). Forward-looking — no current caller demands the looser signature. Filed as TASK-0314 with concrete acceptance criteria.

### Honest scope at AC level

All 4 ACs of TASK-0300 met by the cycle-130 commit + this hardening:
- AC#1 (collect_pair_tiles in backend-common): commit 6e79d0b.
- AC#2 (mp-tcp-bufsync uses it): commit 6e79d0b.
- AC#3 (WalkerCtx backends share the same construction): commit 6e79d0b (pthreads-async + mp-tcp-event both call walker::collect_pair_tiles; WalkerCtx still receives the constructed map as a parameter — that signature is unchanged, only the BUILD is unified).
- AC#4 (pthreads-sync sibling check + benefit): commit 6e79d0b (pthreads-sync also lifted; the cycle-130 expansion also dropped the temp xfer_pairs / pair_tiles rebinding).

### Forward-carry / lessons feed-forward

- For [[TASK-0314]] (P2 #3 follow-up): the new signature must keep existing per_worker.values() call sites unchanged. `Vec<Event>: AsRef<[Event]>` is std, so the migration is no-op at the call sites; the gain is forward-compatibility for a future caller folding a flat slice. Add one new test exercising a `Vec<&[Event]>` input to PROVE the looser signature works (not just compiles).
- For ANY future cycle that further refactors backend-common helpers across the 4 backends: re-run the silent-sibling sweep `grep -rn 'collect_xfer_pairs\|pair_tiles' nucleus/backends/ nucleus/backend-common/` — cycle 130's architect explicitly did this sweep + found zero remaining inline builds; the next refactor MUST start from the same baseline.
- For [[feedback-comment-doc-lie-recurring]]: this cycle's P2 #1 (the '4-line' doc-lie in a NEW file) demonstrates the pattern fires even on freshly-written docstrings — verify count claims against the actual diff, not against narrative recall.

## Cycle 130 status

In Progress → Done after the review-hardening fold-back lands on a green re-gate (108/92/0/16/0 preserved through the hardening commit).
<!-- SECTION:NOTES:END -->
