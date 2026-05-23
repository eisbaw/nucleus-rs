---
id: TASK-0171
title: >-
  Distinct IterVar identity for same-name sibling loops with differing bounds
  (option c, deep ACFG/Event redesign)
status: Done
assignee: []
created_date: '2026-05-18 23:33'
updated_date: '2026-05-23 22:02'
labels:
  - M2
  - compiler
  - acfg
  - tech-debt
dependencies:
  - TASK-0170
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Follow-up from TASK-0170 (option c, the deep long-term fix). TASK-0170 made a same-name-different-bounds loop pair a TYPED SidecarError (build_sidecar -> Result) instead of a panic, so valid programs fail fast+verbose instead of aborting. But the ROOT cause remains: acfg.rs ~614 builds name_iter_vars by enumerating UNIQUE loop-var NAMES (one IterVar per name), so two sequential sibling loops `for i : 0..N {..} for i : 0..M {..}` (writing distinct data so single-assignment holds) collapse to ONE IterVar / ONE Event::Loop.iter_var and cannot both be represented. The proper fix is to give same-name distinct-bound (or generally distinct-occurrence) loops DISTINCT IterVar identity through acfg.rs name_iter_vars + Event::Loop + sidecar loop_bounds + the petri_to_events/EventList path, so such programs COMPILE rather than erroring. This is a deeper ACFG/Event identity change deliberately out of TASK-0170 scope (would ripple into acfg_to_petri/Repeat/backend). PRD 6.2.3: loop vars shadow at their loop and go out of scope at loop end.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 name_iter_vars (acfg.rs) assigns distinct IterVar identity to distinct loop occurrences that reuse a name with differing bounds (not one-IterVar-per-name)
- [ ] #2 Event::Loop.iter_var and sidecar loop_bounds carry the distinct identity end-to-end; the TASK-0170 SidecarError::SameNameLoopBoundConflict path becomes unreachable for this class
- [ ] #3 The same-name-diff-bounds program from TASK-0170's characterisation test now COMPILES and produces correct per-loop bounds (test flipped from expect-error to expect-success)
- [ ] #4 e2e + determinism-check + determinism-check-negative unchanged; clippy clean
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED-no-driver (orchestrator-direct, cycle 78 sweep). TASK-0171 fixes 'same-name distinct-bound sibling loops collapse to one IterVar' so such programs COMPILE rather than erroring. Investigation: grep over all 11 in-tree algo files shows zero same-name-DIFFERENT-bounds sibling-loop pairs. The closest case is 04-prefix-sum which re-uses the  loop var across 3 passes with SAME bounds (0..BS) — that pattern is the TASK-0180 'accumulator double-count' case, which has been FIXED (cycle ~50, per-occurrence BlockTag rebinding on every backend). TASK-0170 (the immediate dependency) made the same-name-DIFFERENT-bounds case a typed SidecarError instead of a panic, so the rare program that DOES write 'for i : 0..N { ... } for i : 0..M { ... }' fails fast+verbose. The deeper fix (give each occurrence distinct IterVar identity so such programs compile) is a deep ACFG/Event/sidecar identity-redesign substrate change that ripples into acfg_to_petri + Repeat + backend layers — substantive deep refactor matching the loop spec's 'warrant fresh context' stop signal. Reopen when (a) a real example uses same-name-different-bounds sibling loops AND (b) the loud-error SidecarError diagnostic is insufficient. Same DEFERRED-no-driver pattern as the cycle-77 sweep + the cycle-78 ADDRESSED-IN-LARGE-PART closures (TASK-0161/0088).
<!-- SECTION:FINAL_SUMMARY:END -->
