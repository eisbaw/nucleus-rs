---
id: TASK-0320
title: >-
  Module-doc/code drift: transfer_inject.rs:161-166 idempotence claim says
  key=(src,dst,data,tile); hoist_invariant_waits dedup key omits tile (architect
  P3-2 forward-carry from TASK-0318 cycle 137)
status: To Do
assignee: []
created_date: '2026-05-25 11:11'
labels:
  - compiler
  - transfer_inject
  - doc-lie
  - comment-doc-lie-recurring
  - forward-carried-from-TASK-0318
dependencies:
  - TASK-0318
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0318 cycle 137 architect P3-2: the module-level docstring at `nucleus/nucleus-compiler/src/passes/transfer_inject.rs:161-166` claims idempotence is by dedup on `(src, dst, data, tile)`. The two actual dedup sites have DIFFERENT keys:

- `inject_in_sequence` (line 887-897): `(src, dst, data, tile)` ✓ matches the doc.
- `hoist_invariant_waits` (line 1172-1178): `(src, dst, data)` only — `tile` is NOT in the match arm.

The module doc overclaims uniformity that the two sites do not have. NOT introduced by cycle 137; predates this cycle (architect spotted during the cycle-137 audit).

## Acceptance criteria

1. Determine whether the divergence is intentional (e.g. `hoist_invariant_waits` runs at a point where two Waits with the same (src, dst, data) but different tiles MUST be merged or are guaranteed identical) or a real defect.
2. If intentional: fix the module-doc to reflect the per-site policy. E.g. 'inject_in_sequence dedups on full (src, dst, data, tile); hoist_invariant_waits dedups on (src, dst, data) because the tile is rebuilt from enclosing_tile.to_vec() the moment before the dedup check, so any two Waits surviving to this point have identical tiles by construction.'
3. If a real defect: file the concrete shape (e.g. 'two cross-worker Waits on the same data with different tile granularities would be silently merged') as a separate code task and pin the divergence as a regression test in transfer_inject_hoist.rs or hoist_invariant_waits's test fixture.

## Honest scope

- LOW priority. Pre-existing drift; cycle-137 surfaced it but did not introduce it. No e2e regression observed under shipped schedules (every shipped Wait pair has a unique (src, dst, data) per scope under the M5 partition shapes).
- Trigger: next M6+ schedule that produces two cross-worker Waits on the same data with different tile granularities, OR a quality-coverage cycle that audits the module doc against the code.

## Cross-reference

- transfer_inject.rs:161-166 (module doc, the claim).
- transfer_inject.rs:887-897 (inject_in_sequence dedup, includes tile).
- transfer_inject.rs:1172-1178 (hoist_invariant_waits dedup, omits tile).
- TASK-0318 cycle 137 architect P3-2 (the surfacing review).
<!-- SECTION:DESCRIPTION:END -->
