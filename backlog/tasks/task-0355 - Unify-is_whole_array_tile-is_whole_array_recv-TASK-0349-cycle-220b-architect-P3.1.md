---
id: TASK-0355
title: >-
  Unify is_whole_array_tile + is_whole_array_recv (TASK-0349 cycle 220b
  architect P3.1)
status: To Do
assignee: []
created_date: '2026-05-27 23:58'
updated_date: '2026-05-28 00:36'
labels:
  - backend-common
  - refactor
  - opacity-gate-rot-adjacent
  - cycle-220b-follow-up
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-220 architect P3.1: two whole-array-vs-slice classifiers exist in backend-common with similar but not identical semantics:

- collect.rs:260 `is_whole_array_tile` — predates TASK-0349; returns bool on empty bounds, scalar ty, full-range bounds; returns false on axis-beyond-dims (silently).
- wait.rs:369 `is_whole_array_recv` — TASK-0349 cycle 220; wraps `wait_slice` which has rank-guard Err arms, both-axes-full -> None, etc.

For the common shipped cases they agree; for edge cases they may diverge. This is **opacity-gate-adjacent** per memory feedback-opacity-gate-rot — the two classifiers can drift independently as one or the other evolves to handle new tile/data shapes.

## Acceptance

1. Pick one canonical classifier (likely `is_whole_array_recv` since it routes through wait_slice's shape-error invariants).
2. Migrate the call site of the other to route through the canonical one.
3. Remove the deprecated classifier.
4. Add a sibling test that exercises the edge-case shapes (rank > 2, out-of-bounds, scalar, empty bounds) to pin the unified semantics.

## Honest scope

Refactor; no behaviour change for shipped cells. Low priority because both classifiers happen to agree on every currently-shipped tile/data shape.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0354 (cycle 221): the 7 new unit pins for collect_let_at_wait_data exercise is_whole_array_recv across these (data, tile) shapes; reuse / extend them when designing the unified classifier's test suite:

- Whole-array via tile-bounds-empty (IterTile::empty()) → Ok(None) → recv-whole — tests 1, 2, 3, 4, 6.
- Whole-array via no-pair_tiles-entry (collect.rs:389-390 None arm) → recv-whole, even when other tiles exist on different SeqTags — test 1.
- Slice via 1D tile NOT covering leading-dim — test 1's slice arm.
- Out-of-bounds leading-axis range (start<0 OR end>leading_dim OR start>=end) → wait_slice:269-278 ContractGap Err — test 5. This is the silent-defensive arm: is_whole_array_recv propagates Err; collect_let_at_wait_inner:392 swallows with .unwrap_or(false). Either the unified classifier preserves this 'Err = not whole' semantic OR it surfaces the Err to callers — DECISION NEEDED at TASK-0355 design time.
- Scalar data (dims=[]) with non-empty tile bounds → wait_slice:265-267 early-return Ok(None) → whole — test 7. Pins the ordering: empty-dims check MUST precede ty.dims[0] read.

Sibling-divergence finding (the keystone fact for TASK-0355 per the task brief's forward-carry guidance): is_whole_array_recv currently goes through wait_slice's FULL guard chain (rank-3+ guard at :307, out-of-bounds at :269-278/:324-327, sidecar lookup at :259-263) — i.e. it conservatively classifies any shape wait_slice can't handle as 'not whole'. If is_whole_array_tile lives elsewhere and applies a SUBSET of those guards (e.g. only checks bounds-len > 0), unifying them changes the Err-on-shape-X behaviour at one of the call sites. Audit the existing is_whole_array_tile call sites for which Err arms they tolerate before swapping in a unified helper.

Visibility note: is_whole_array_recv is pub(super) (narrowed cycle 220b architect P2.2). If the unified helper is moved to a shared location, keep visibility minimal (pub(crate) at widest); no out-of-crate consumer exists today and adding one without need re-opens the surface architect P2.2 just closed.
<!-- SECTION:NOTES:END -->
