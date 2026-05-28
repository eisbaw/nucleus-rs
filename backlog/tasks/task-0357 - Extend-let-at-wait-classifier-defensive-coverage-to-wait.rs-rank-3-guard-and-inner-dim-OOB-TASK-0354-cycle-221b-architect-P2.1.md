---
id: TASK-0357
title: >-
  Extend let-at-wait classifier defensive coverage to wait.rs rank-3+ guard and
  inner-dim OOB (TASK-0354 cycle-221b architect P2.1)
status: Done
assignee:
  - '@orchestrator'
created_date: '2026-05-28 00:48'
updated_date: '2026-05-28 01:23'
labels:
  - tests
  - backend-common
  - defensive
  - cycle-221b-follow-up
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-221b architect P2.1: TASK-0354 cycle-221 added 7 unit tests for `collect_let_at_wait_data` but only 3 of them (tests 1, 5, 7) actually reach the `is_whole_array_recv` -> `wait_slice` code path. Tests 2, 3, 4, 6 all use empty `pair_tiles` so the `None` arm at `collect.rs:389-390` fires and `wait_slice` is never invoked.

The following `wait_slice` guards are therefore **structurally unexercised** by the new test file:

1. Rank-3+ guard at `wait.rs:307-320` — `Err(ContractGap)` when `tile.bounds.len() > 2` or `tile.bounds.len() >= 2 && ty.dims.len() > 2`. The cycle-209 16-jacobi/distributed unblocker explicitly cited this as the next-layer blocker. TASK-0341.02.02.01.01 lifts to N-D nested-loop dispatch; until then the guard fires LOUD for any rank-3 shape.

2. Inner-axis OOB at `wait.rs:324-333` — `Err(ContractGap)` when `inner_range.start < 0`, `inner_range.end > inner_dim`, or `inner_range.start >= inner_range.end`. Symmetric to the leading-axis OOB guard test 5 already covers.

## Acceptance

1. Add a `rank_3_tile_shape_excludes_data` test driving `collect_let_at_wait_data` with a rank-3 tile (3 bounds entries) on rank-2 data, OR a 2-bound tile on rank-3 data. Confirm `wait_slice:307` fires `Err`, `.unwrap_or(false)` swallows, data excluded.

2. Add an `inner_axis_oob_excludes_data` test with a 2-bound tile on rank-2 data where the inner range exceeds `dims[1]`. Confirm `wait_slice:324-333` fires `Err`, data excluded.

3. Module docstring updated to enumerate the new cases as items 8 + 9 below the existing 1-7 list.

## Honest scope

Test-only; no production-code edits. Closes the silent-sibling gap the architect flagged in TASK-0354's review.

## Dependencies

- Builds on TASK-0354 (which lands cases 1-7).
- Adjacent to TASK-0355 (unify is_whole_array_tile + is_whole_array_recv) — the unified classifier would subsume both; if TASK-0355 is picked first, this task's content moves into the unified test suite.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Cycle 223 plan — orchestrator-in-thread (test-only, no implementer spawn):

1. Add `rank_3_tile_shape_excludes_data` test:
   - Construct rank-3 tile (3-bound tile_3d helper or inline) on rank-2 data dims=[16,16].
   - pair_tiles entry maps (data, seq) -> rank-3 tile.
   - Wait event with rank-3 tile.
   - Assert data excluded (wait_slice:307 rank-3+ guard fires Err, .unwrap_or(false) at collect.rs:392 propagates as "not whole" -> not_all_whole -> excluded).

2. Add `inner_axis_oob_excludes_data` test:
   - dims=[8,8] on rank-2 data; tile with bounds (iv0, 0..8) and (iv1, 0..1024). Inner range 0..1024 > dim[1]=8.
   - wait_slice:324-333 inner-axis OOB guard fires Err.
   - Assert data excluded.

3. Update module docstring: append items 8 + 9 below the existing 1-7 list, with one-line descriptions and citations to wait.rs guard line numbers.

Gate verification (cheap subset before commit):
- `nix develop --command bash -c "just test && just clippy"` — expect +2 tests passing (1026 -> 1028) on dev, +2 on release (1025 -> 1027), clippy clean.
- `just e2e` baseline preserved at 280/246/0/34/0 (test-only, structurally cannot regress).

Forward-carry to TASK-0355 (unify is_whole_array_tile + is_whole_array_recv): the 2 new tests cover the FULL guard chain of is_whole_array_recv (now: rank-3+, both axes OOB, scalar, mixed-slice). When TASK-0355 is picked up, the unified classifier MUST exercise the same 9 cases (cycle 221's 7 + cycle 223's 2).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Cycle 223 closure — Done after parallel review gate landed double-GO.

Gate (orchestrator pre-commit + qa-test-runner independent re-verification + mped-architect read-only):
- cargo test (dev): 1026/0/3 -> 1028/0/3 (+2 exactly, both reviewers confirmed).
- cargo test (release): 1025/0/3 -> 1027/0/3 (+2 exactly).
- just clippy --all-targets -D warnings: clean.
- just e2e: 280/246/0/34/0 bit-identical across 2 independent runs (non-flake).
- 4 structural checks (textual-replace, include-str, narrative-doc-lie, mega-files): all clean.

Architect P1 = none (all claims verified: wait.rs:307 LEFT disjunct fires for test 8; wait.rs:324 inner-axis OOB fires for test 9; tile_3d arg order verified; module docstring + commit msg cross-check).

Architect P2 = 2 honest-not-silent deferrals (no action):
  - Test 8 right-disjunct (rank-2 tile on rank-3 data): empirically structurally equivalent (same Err propagation, same .unwrap_or(false) arm).
  - Test 9 rejected degenerate-range arm: symmetric to OOB, same arm.

Architect P3.2: tile_2d + tile_3d helpers join the shared-helper migration scope — forward-carried to TASK-0358 (commit d3719e8's filed task).

Architect P3.3: TASK-0357's AC formalization deliberately NOT done in this cycle (per feedback-ac-rewrite-on-done-task: avoid AC-rewrite on a near-Done task). Tracker-hygiene sweep candidate for a future cycle.

Commits: c45a5e4 (cycle 223 implementation) + TASK-0358 forward-carry md edit being committed alongside this closure note.
<!-- SECTION:NOTES:END -->
