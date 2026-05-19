---
id: TASK-0180
title: >-
  block= over a loop variable reused across multiple passes skips absolute-index
  rebinding (accumulator double-counts)
status: To Do
assignee: []
created_date: '2026-05-19 01:18'
updated_date: '2026-05-19 01:18'
labels:
  - M3
  - backend
  - findings
dependencies:
  - TASK-0039
  - TASK-0173
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Surfaced by TASK-0039 (example 04-prefix-sum, blocked schedule). divisible_inner_block_vars (nucleus/backends/pthreads-sync/src/lib.rs ~455) only grants absolute-index rebinding to an inner-block IterVar whose loop appears EXACTLY ONCE in the EventList (counts==1). That count==1 guard exists to avoid the non-divisible full+partial two-nest ambiguity (TASK-0173). But it ALSO excludes a loop variable NAME legitimately reused across several independent passes: example 04 has three passes each  (NB=4) with  (EVENLY divisible, no remainder). block_transform reuses b's IterVar for all three inner loops, so counts[b]==3, b is dropped from divisible_inner, NO rebinding is applied, and the inner loop runs the FULL source range while wrapped by the tile loop => each accumulator body executes tiles*range times instead of range times. 04-prefix-sum/blocked output is exactly 2x the correct prefix sums on BOTH backends. 05-stencil/07-matmul don't hit this because each uses its tiled var in exactly one loop. Root issue: the count==1 heuristic conflates 'this IS the divisible single-nest' with 'this name is reused across passes'. Fix: distinguish divisible single-nest inner vars structurally (e.g. block_transform tags each inner loop with its (lo, num_full, partial?) so the backend can rebind per-occurrence) rather than by a global occurrence count. Until fixed, a blocked schedule over any algorithm that reuses a loop var name across passes (esp. accumulators) is WRONG. Mitigation in 04: blocked schedule shipped but [[skip]]'d with this reason; only naive is a required differential cell.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 block_transform tags each strip-mined inner loop so the backend rebinds per-occurrence (not by global EventList count)
- [ ] #2 A blocked schedule over a loop var reused across >=2 passes (accumulator) is bit-identical to its naive schedule on both backends
- [ ] #3 04-prefix-sum/blocked moves from [[skip]] to [[required]] for both backends; existing blocked cells (05,07) stay green
- [ ] #4 block_transform tags each strip-mined inner loop so the backend rebinds per-occurrence (not by global EventList occurrence count)
- [ ] #5 A blocked schedule over a loop var reused across two or more passes (accumulator) is bit-identical to its naive schedule on both backends
- [ ] #6 04-prefix-sum blocked moves from skip to required for both backends; existing blocked cells (05, 07) stay green; determinism stays green
<!-- AC:END -->
