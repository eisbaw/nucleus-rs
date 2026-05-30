---
id: TASK-0366
title: >-
  rewrite_cumulative_band_tiles: upgrade silent None->whole-array fallback to
  fail-loud EmitError for a partitioned cumulative symbol
status: To Do
assignee: []
created_date: '2026-05-30 09:53'
labels:
  - compiler
  - transfer_inject
  - M6
  - 16-jacobi
  - fail-loud
  - cycle-213-foldback
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-213 architect P3 fold-back. In transfer_inject.rs::rewrite_cumulative_band_tiles, when a CUMULATIVE data symbol (NameSidecar::cumulative_data) has a transfer for which cumulative_band_bounds() returns None, the tile is left unchanged (whole-array). For a cumulative array a whole-array transfer silently re-introduces the xN double-count the pass removes. Provably dead today (16-jacobi field always derives a write band; game-of-life ships no partitioned schedule) and the e2e bit-identity differential would catch it, so cycle 213 made it OBSERVABLE via nuc_trace! only. This task upgrades it to a fail-loud EmitError (the transfer_inject pass entry already returns Result, so the rewrite can be made fallible) so a future partitioned-cumulative shape that hits the None branch fails at compile time instead of emitting xN-wrong output. Reference: nuc_trace! site in rewrite_cumulative_band_tiles + the cumulative_band_bounds None branch.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 rewrite_cumulative_band_tiles is made fallible (or wrapped) so a None from cumulative_band_bounds on a symbol in cumulative_data raises EmitError::ContractGap instead of silently leaving a whole-array tile
- [ ] #2 A negative unit test constructs a cumulative Xfer whose src has no band (None path) and asserts the typed error fires
- [ ] #3 16-jacobi/distributed stays bit-identical (the dead branch is never hit by shipped schedules; e2e total unchanged)
<!-- AC:END -->
