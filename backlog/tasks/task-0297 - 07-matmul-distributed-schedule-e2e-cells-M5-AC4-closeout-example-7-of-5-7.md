---
id: TASK-0297
title: >-
  07-matmul distributed schedule + e2e cells (M5 AC#4 closeout, example 7 of
  5-7)
status: Done
assignee:
  - '@mped-orchestrator'
created_date: '2026-05-25 00:49'
updated_date: '2026-05-25 02:23'
labels:
  - M5
  - compiler
  - partition
  - blocks2d
  - distributed
  - 07-matmul
dependencies:
  - TASK-0301
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

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
ORCHESTRATOR-DIRECT IMPLEMENTATION (cycle 117, post-TASK-0296 cycle-116).

Plan:
1. Read 07-matmul/prog.algo.nuc, existing 05-stencil/distributed-2d.sched.nuc (template), partition_blocks2d.rs, transfer_inject behavior for col-band (trailing-axis) gather on b.
2. Decide partition shape: partition=blocks2d on i (outer of {i, j, k}). Each worker (row_w, col_w) computes C[i_band][j_band].
3. Trace data movement:
   - a[i][k]: row-band slice (a[i_band][full k]) — 1D leading-axis slice (TASK-0117 cycle-79 path)
   - b[k][j]: col-band slice (b[full k][j_band]) — trailing-axis. The cycle-115 2D row-loop path is LEADING+INNER for rank-2-on-rank-2; col-band is a different shape. Either transfer_inject handles it OR needs broadcast/honest-scope.
   - c[i][j]: 2D rectangle gather (c[i_band][j_band]) — exactly the cycle-115 2D row-loop slice-paste shape.
4. Write 07-matmul/distributed.sched.nuc, attempt minimal sync shape (broadest backend reach).
5. Build manually for each backend; inspect emit; iterate.
6. Add e2e cells; run gate; iterate to bit-identical.
7. Honest scope: if col-band on b doesn't work, ship the shape that does (e.g., partition=workers on i only, with broadcast on b) and file precise follow-ups.

Honest-failure path: if 2D matmul + partition=blocks2d hits codegen gaps in transfer_inject for b's col-band, file the precise gap as a prerequisite, ship a simpler shape (1D partition on i with broadcast on b), and leave M5 AC#4 example-7 as half-done with a clear next-step trail.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
CLOSED cycle 118 (2026-05-25). Blocker TASK-0301 LANDED — additive per-data axis-mapping filter at transfer_inject.rs::rewrite_partition_tiles_inner correctly excludes the partitioned i from b's tile bounds (b is indexed [k][j], not by i) → empty bounds → wait_slice's whole-array arm broadcasts full b. All 4 tier-1 backends pass bit-identical:

  07-matmul/distributed × pthreads-sync   PASS (847ms)
  07-matmul/distributed × mp-tcp-bufsync  PASS (803ms)
  07-matmul/distributed × pthreads-async  PASS (1.03s)
  07-matmul/distributed × mp-tcp-event    PASS (3.10s)

PROMOTED: 4 [[skip]] → 4 [[required]] M5 in e2e-matrix.toml.
SCHEDULE: examples/07-matmul/schedules/distributed.sched.nuc — BLOCKED header replaced with production comment.

M5 AC#4 COMPLETE for examples 5-7 (PRD §11): examples 5 (05-stencil, cycles 79c..115), 6 (06-separable-filter, cycle 116), 7 (07-matmul, cycle 118) all have distributed schedules landing bit-identical on at least one tier-1 backend.

Baseline 104/84/0/20/0 → 104/88/0/16/0.
<!-- SECTION:NOTES:END -->
