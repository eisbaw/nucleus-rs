---
id: TASK-0355
title: >-
  Unify is_whole_array_tile + is_whole_array_recv (TASK-0349 cycle 220b
  architect P3.1)
status: In Progress
assignee:
  - '@mark'
created_date: '2026-05-27 23:58'
updated_date: '2026-05-28 02:21'
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

## Cycle 225 implementation plan (orchestrator-direct)

Picked TASK-0355 as the next-cycle task per user direction 'continue with structurally-different' (TASK-0344 cite-sweep just landed; this is semantic classifier consolidation, structurally distinct defect surface).

### Design decision

**Canonical classifier: `is_whole_array_recv` (wait.rs:376).**

Rationale per the cycle-220b architect's P3.1 forward-carry + the task brief's analysis:
- Routes through wait_slice's established shape-error invariants (rank-3+ guard at wait.rs:307; OOB at wait.rs:269-278/324-327; sidecar lookup at :259-263).
- The deprecated `is_whole_array_tile` (collect.rs:260) silently returns false on axis-beyond-dims; the unified classifier surfaces this as an explicit `Err` arm. Callers choose Err semantics.

**Err semantics: swallow with `.unwrap_or(false)` at both call sites.**

Rationale: matches the existing collect.rs:392 site's swallow convention. Conservative: Err = 'shape wait_slice can't classify' = treat as not-whole-array. Forward-compatible: when a future wait_slice extension (e.g. TASK-0341.02.02.01.01's N-D dispatch) opportunistically classifies a previously-Err shape, the migration auto-benefits without site-by-site updates.

Alternative considered + rejected: surfacing Err to callers would change behaviour at collect.rs:209's accumulator-detection arm (currently never sees Err; would now propagate Err up). The brief's accumulator-detection ContractGap surfaces 'later in the emit path' per the existing comment — keeping that contract.

### Steps

1. Migrate collect.rs:209 call from `is_whole_array_tile(tile, ty)` to `super::wait::is_whole_array_recv(sidecar, data, tile).unwrap_or(false)`. Thread `data` (already in scope from line 198 `for (data, seqs)`).
2. Remove `ty` binding at collect.rs:202-205 (no longer needed; is_whole_array_recv looks up sidecar internally).
3. Remove `is_whole_array_tile` fn from collect.rs (lines ~250-282).
4. Add tests/whole_array_classifier.rs exercising:
   - Whole via empty bounds → true
   - Whole via scalar (empty dims) + non-empty bounds → true (wait_slice:265 early-return)
   - Slice (non-full leading range, rank-1) → false
   - 2D both-axes-full → true
   - Rank-3+ shape → Err (sanity-check that wait.rs:307 guard fires; verifies the migration's conservative semantic)
   - OOB leading range → Err
5. Run gate: build, clippy, test, test-release, e2e. e2e baseline 280/246/0/34/0 must hold bit-identical (no shipped schedule trips a divergence between the two classifiers per cycle-220b architect P3.1 narrative; behaviour change is invisible to e2e gate).
6. Commit. Then parallel reviewer gate (qa + architect).
<!-- SECTION:NOTES:END -->
