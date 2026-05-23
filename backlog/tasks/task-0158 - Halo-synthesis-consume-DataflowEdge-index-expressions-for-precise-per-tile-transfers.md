---
id: TASK-0158
title: >-
  Halo synthesis: consume DataflowEdge index expressions for precise per-tile
  transfers
status: Done
assignee: []
created_date: '2026-05-18 13:41'
updated_date: '2026-05-23 21:29'
labels:
  - followup
  - compiler
  - M2
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0150 plumbed per-firing index expressions onto DataflowEdge (data_in_access / data_out_access). transfer_inject still hoists whole-symbol cross-worker transfers by STRUCTURAL loop-invariance (data not produced inside the intra-tile body), which over-transfers a full tile where only a halo strip is needed for stencil-like access (e.g. img_in[y-1][x] under a y-blocked loop). This task consumes the now-available index expressions to synthesise the actual per-tile halo-band IterTile so a blocked/distributed stencil transfers only the needed strip. Coupled with TASK-0117 (distributed placement): halo only matters once data is partitioned across workers; verify against 05-stencil distributed/blocked once TASK-0117 lands. Conservatively safe today (over-serialise only), so this is a precision/performance upgrade, not a correctness fix.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 transfer_inject derives per-tile halo IterTile from DataflowEdge.data_in_access index expressions (affine offsets over iter vars + consts) instead of the whole-symbol structural hoist, for block= and distributed stencil access
- [ ] #2 Non-affine / data-dependent indices are rejected at compile time with a named symbol (PRD §3, §8.6 — affine only), not silently mis-transferred
- [ ] #3 05-stencil blocked and distributed (post TASK-0117) transfer only the halo band; bit-identical e2e preserved for all existing cells; determinism preserved
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED-precision-not-correctness (orchestrator-direct, cycle 77 sweep). Task self-describes: 'Conservatively safe today (over-serialise only), so this is a precision/performance upgrade, not a correctness fix.' The over-transfer of a full tile instead of a halo strip is benign behavior — it wastes bandwidth on stencil-class examples but doesn't change output bytes. No perf measurement has shown the over-transfer bites (05-stencil/distributed is currently SKIPPED across all 4 tier-1 backends per TASK-0117/0181/0042.05 — when it becomes [[required]], the bandwidth waste could matter). Plus the dependency TASK-0150 (per-firing index expressions on DataflowEdge) — status unknown but referenced as 'plumbed' in past tense; this task is the consumer that hasn't followed up. Reopen when (a) 05-stencil/distributed becomes [[required]] on at least one backend AND (b) the bandwidth waste of whole-tile transfers becomes measurable. Same deferred-precision pattern.
<!-- SECTION:FINAL_SUMMARY:END -->
