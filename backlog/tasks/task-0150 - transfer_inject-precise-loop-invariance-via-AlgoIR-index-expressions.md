---
id: TASK-0150
title: 'transfer_inject: precise loop-invariance via AlgoIR index expressions'
status: To Do
assignee: []
created_date: '2026-05-18 05:45'
updated_date: '2026-05-18 09:35'
labels:
  - followup
  - compiler
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0143 hoists Waits structurally: any data not produced inside the intra-tile body is treated as invariant w.r.t. the inner iteration. This is conservative; for a stencil pattern such as img_in[y][x] read inside a y-blocked inner loop, the access depends on y so the data is NOT actually y-invariant — yet the structural rule hoists anyway because the producer (load_image) lives above the tile loop. This is fine for correctness (the full data ranges arrive once per tile and the consumer reads the right slice within the tile) but it means the hoisted Push carries one *full* tile of data per outer-tile iteration even when only a halo strip is actually needed. To get tighter per-tile transfers (halo synthesis) the pass needs access-pattern info: re-plumb IndexExpr through DataflowEdge so transfer_inject can synthesise the actual y-band slice. Coupled with TASK-0117 (distributed placements) since halo only matters when the data is partitioned.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Context from TASK-0136/0151/0153: the current whole-symbol cross-scope model uses STRUCTURAL loop-invariance (data not produced inside the loop body) instead of precise AlgoIR index analysis. This is conservatively SAFE, not a bug: it can only OVER-serialise (treat an index-varying access as a whole-symbol crossing, transferring more/serialising more than strictly necessary) — it never under-synchronises and never collapses a genuinely per-iteration transfer into one. TASK-0150 is the precision upgrade (per-firing index expressions through ACFG so e.g. disjoint a[i] slices can pipeline); until it lands, the over-serialisation is a performance ceiling, not a correctness defect. Also relevant: TASK-0151 over-approximation (block-entangled non-block transfers stranded) and the single-assignment debug-assert added in TASK-0153.
<!-- SECTION:NOTES:END -->
