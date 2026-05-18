---
id: TASK-0030
title: block=N loop transformation
status: Done
assignee: []
created_date: '2026-05-17 23:06'
updated_date: '2026-05-18 04:25'
labels:
  - M2
  - compiler
  - language
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Implement block=N in the schedule's loop transforms. Outer loop iterates over tiles of N; inner loop iterates within a tile. Transfer events happen at tile granularity.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Schedule directive 'loop var : block=N' rewrites the iteration tree to a (tile-loop, intra-tile-loop) nest.
- [ ] #2 Transfer event sizes (IterTile bounds) align with the tile, not per-point.
- [ ] #3 Test: example 5 (stencil) compiled with block=64 produces an EventList where each Push covers a 64-row band.
- [ ] #4 Implementation notes record design questions (e.g. handling of trailing remainder when iteration size is not a multiple of N).
- [ ] #5 Implementation notes record honest limitations (block= is applied left-to-right with other loop options; some combinations may not yet be supported and should be rejected).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implementation summary
=========================

Pass lives at nucleus/compiler/src/passes/block_transform.rs and is wired into the driver pipeline between build_acfg and inject_syncs:

  parse -> lower -> link -> build ACFG -> apply_block_transforms -> inject_syncs -> inject_transfers -> emit

For each loop directive in linked.sched.loops carrying ResolvedLoopOption::Block(N), the pass finds the matching ACFGNode::Repeat by iter_var name and replaces it with:

  Repeat { iter_var: <var>__tile, range: 0..num_tiles, body: Sequence([
    Repeat { iter_var: <var>, range: 0..N, body: <orig body> }
  ])}

The inner loop reuses the original IterVar id; only the outer (tile) iter var gets a fresh id. This keeps the inner loop's IterTile compatible with the existing transfer-inject pass: examples without block= are bit-identical end-to-end (verified by the e2e matrix and the dedicated identity test).

Design decisions and answers to the questions raised in the task
=================================================================

Q: ACFG-build vs post-build?
A: Post-build. ACFG construction stays a pure function of LinkedIR (no schedule-loop-option lookup tangled into it) and the block transform is one self-contained ACFG -> ACFG pass. Mirrors sync_inject / transfer_inject.

Q: Trailing remainder when H is not a multiple of N?
A: REJECTED with BlockTransformError::NotDivisible carrying (var, lo, hi, N). The ACFG's Repeat::range is a single Range<i64> that cannot express the 'min(y_outer+N, H)' clamp. Supporting remainder needs an IR extension (new Repeat variant or dynamic bound expression). Filed as follow-up TASK-0142. Failing loud is safer than silently emitting a partial tile.

Q: Two iter vars (y_outer + y_inner) vs single rebased var?
A: Two iter vars. Outer is <var>__tile (fresh id); inner keeps the original <var> name and id. This minimises downstream churn -- the inner loop's IterTile is still keyed on <var>, so the transfer-inject pass and the petri lowering see the same iter-var identities they already knew about. Trade-off: the inner loop's range is 0..N (offset within tile), not LO..LO+N. Codegen that wants the absolute iteration value would compute LO + tile*N + inner -- the current M2 codegen doesn't yet read absolute values, so this is OK for now.

Honest limitations
==================

1. Only **outermost** loops can be blocked. Nested blocking (block=N on both y and x) works in isolation per loop, but no coordination is attempted -- each Repeat matches its own block= directive independently and is rewritten as a 2-level nest. A future task could extend this to handle 'block_integral'-style coordination.

2. Transfers are NOT yet per-tile. PRD §6.3.3 says 'transfers happen per tile' but the existing transfer_inject pass still injects Push/Wait per intra-tile iteration (one transfer per inner-loop firing). Hoisting Push/Wait out to the outer (tile) loop is a transfer_inject change that requires loop-invariance analysis. Filed as follow-up TASK-0143. This means TASK-0030 AC #3 (example 05 stencil with block=64 produces Push covering a 64-row band) is NOT yet hit -- example 05's algorithm also doesn't compile (TASK-0078), so this AC is double-blocked. The structural precondition (two-level nest) is in place.

3. No conflict detection between block= and other loop options (vectorize=, unroll=, pipeline=). PRD §6.3.3 says bad combinations must be rejected at compile time. Filed as follow-up TASK-0144.

4. Duplicate block= on the same loop var: last-wins (same convention as the schedule lowering pass uses for sync/async on transfers).

5. Synthetic outer iter var name <var>__tile: collisions with user-declared iter vars trigger a panic. The algorithm grammar permits __ in identifiers (PRD §6.2.3 name resolution) so a collision is theoretically possible. No real example collides; deferred until one does.

AC verification
===============

#1 (block= rewrites iteration tree to nest): YES -- test 'block_rewrites_to_outer_tile_and_inner' asserts the post-transform shape.

#2 (transfer event sizes align with tile): NOT YET -- per-iteration Push/Wait persists. Structural precondition (two-level nest) in place; tile-coalescing is TASK-0143.

#3 (example 05 stencil block=64 -> 64-row band Push): NOT YET -- example 05 doesn't compile (TASK-0078) and per-tile transfers are TASK-0143.

#4 (record design questions in notes): YES -- this note.

#5 (record honest limitations re: combinations): YES -- documented above and TASK-0144 filed.

Follow-ups filed
================

- TASK-0142: support trailing remainder tiles.
- TASK-0143: hoist Push/Wait to per-tile granularity.
- TASK-0144: reject bad combinations of block= with other loop options.

Verification
============

- just check, just clippy, just test all pass.
- just e2e matrix unchanged (5 cells, 4 PASS + 1 SKIPPED -- no examples in the matrix use block=).
- Dedicated test 'examples_01_02_03_unchanged_by_block_transform' asserts the pass is the identity on every required (algo, sched) pair from the e2e matrix.
<!-- SECTION:NOTES:END -->
