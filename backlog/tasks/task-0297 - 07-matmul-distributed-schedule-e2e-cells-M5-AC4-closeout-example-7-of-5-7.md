---
id: TASK-0297
title: >-
  07-matmul distributed schedule + e2e cells (M5 AC#4 closeout, example 7 of
  5-7)
status: To Do
assignee: []
created_date: '2026-05-25 00:49'
updated_date: '2026-05-25 00:50'
labels:
  - M5
  - compiler
  - partition
  - blocks2d
  - distributed
  - 07-matmul
dependencies:
  - TASK-0296
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background
PRD §11 M5 acceptance: "Examples 5–7 benefit measurably" from distributed/reuse. This task files the M5-extension distributed schedule for example 7-matmul. Sibling task: TASK-0296 (06-separable-filter distributed).

## What example 7 looks like
Blocked integer matrix multiply (see nuc-nucleus/examples/07-matmul/prog.algo.nuc):
```
for i : 0 .. N {
  for j : 0 .. N {
    for k : 0 .. N {
      c[i][j] <-- madd(c[i][j], a[i][k], b[k][j]);
    }
  }
}
```
The example comment itself calls out: "All-to-all communication when distributed: every worker computing a tile of C needs at least one full row of A and one full column of B."

## Honest scope concerns
1. **Natural shape**: partition=blocks2d on outer (i, j) — each worker owns a tile of C[i_band][j_band]. This is the classic 2D-block matmul layout.
2. **Transfer pattern is broadcast/all-to-all, NOT halo**:
   - Worker (row, col) reads a[i_band][...] (full rows of A for its row band) and b[...][j_band] (full columns of B for its col band).
   - Halo inference does NOT apply (k iterates 0..N for every output; no bounded local-neighbourhood pattern).
   - transfer_inject needs to handle row-band gather on A + col-band gather on B per worker. This may already be supported by the partition=rows machinery applied across two independent axes; verify.
3. **Cycle 115 2D slice-paste machinery applies on the OUTPUT side**: c is 2D-tiled, the host gather pastes per-worker rectangles into the global c. The new WaitSlice::Rows variant (cycle 115) is needed exactly for this case.

## Acceptance criteria
1. nuc-nucleus/examples/07-matmul/schedules/distributed.sched.nuc exists, uses partition=blocks2d on outer (i, j), lowers end-to-end on a tier-1 backend bit-identical to reference.bin.
2. New e2e cell in nuc-nucleus/e2e-matrix.toml: 07-matmul / distributed / <backend> tagged milestone=M5, [[required]].
3. Sibling backends documented as [[skip]] with cited blockers — likely TASK-0042 (if async transfer is needed) and TASK-0175 (if any host-excluding barriers emerge).
4. Schedule comment explains the chosen partition shape, transfer pattern (broadcast vs halo distinction), and why no halo inference is involved.
5. e2e gate still green; baseline advances by ≥1 cell.

## Cross-references
- nuc-nucleus/examples/05-stencil/schedules/distributed-2d.sched.nuc — template for partition=blocks2d shape.
- nucleus/nucleus-compiler/src/passes/partition_blocks2d.rs (TASK-0259) — the consumer.
- nucleus/backend-common/src/multi_worker_walker.rs WaitSlice::Rows variant (TASK-0294 cycle 115) — 2D-tile host-gather codegen.
- nucleus/nucleus-compiler/src/passes/transfer_inject.rs — verify behaviour for unbounded `k` accesses (broadcast, not halo).

## Dependency
- May depend on TASK-0296 finishing first if the implementer needs to validate the broadcast-not-halo machinery on the simpler example 6 case before attacking the 2D version here.
<!-- SECTION:DESCRIPTION:END -->
