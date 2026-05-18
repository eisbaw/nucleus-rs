---
id: TASK-0171
title: >-
  Distinct IterVar identity for same-name sibling loops with differing bounds
  (option c, deep ACFG/Event redesign)
status: To Do
assignee: []
created_date: '2026-05-18 23:33'
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
