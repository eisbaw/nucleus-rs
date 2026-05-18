---
id: TASK-0150
title: 'transfer_inject: precise loop-invariance via AlgoIR index expressions'
status: To Do
assignee: []
created_date: '2026-05-18 05:45'
labels:
  - followup
  - compiler
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0143 hoists Waits structurally: any data not produced inside the intra-tile body is treated as invariant w.r.t. the inner iteration. This is conservative; for a stencil pattern such as img_in[y][x] read inside a y-blocked inner loop, the access depends on y so the data is NOT actually y-invariant — yet the structural rule hoists anyway because the producer (load_image) lives above the tile loop. This is fine for correctness (the full data ranges arrive once per tile and the consumer reads the right slice within the tile) but it means the hoisted Push carries one *full* tile of data per outer-tile iteration even when only a halo strip is actually needed. To get tighter per-tile transfers (halo synthesis) the pass needs access-pattern info: re-plumb IndexExpr through DataflowEdge so transfer_inject can synthesise the actual y-band slice. Coupled with TASK-0117 (distributed placements) since halo only matters when the data is partitioned.
<!-- SECTION:DESCRIPTION:END -->
