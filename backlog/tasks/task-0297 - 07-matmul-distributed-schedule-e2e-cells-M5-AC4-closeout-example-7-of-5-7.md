---
id: TASK-0297
title: >-
  07-matmul distributed schedule + e2e cells (M5 AC#4 closeout, example 7 of
  5-7)
status: In Progress
assignee:
  - '@mped-orchestrator'
created_date: '2026-05-25 00:49'
updated_date: '2026-05-25 01:36'
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
CYCLE 117 BLOCKED (orchestrator-direct, 2026-05-25):
- Drafted nuc-nucleus/examples/07-matmul/schedules/distributed.sched.nuc with partition=workers on i, sync transfers for a/b/c.
- Smoke-built on pthreads-sync via release driver; emit inspection found:
  - a: `a[0..64].copy_from_slice(&_tmp[0..64])` — CORRECT (a indexed [i][k], leading axis i = partitioned).
  - b: `b[0..64].copy_from_slice(&_tmp[0..64])` — WRONG. b indexed [k][j]; leading axis k. tile.bounds[0] = (i, i_band) silently sliced b's k axis by i_band. Worker 0 receives only b[k=0..i_band.end][full j], reads zero-default for k beyond its band.
  - c: gather correct (c indexed [i][j], leading axis i).
- ROOT CAUSE: AXIS-MAPPING ASSUMPTION limit documented at `nucleus/backend-common/src/multi_worker_walker.rs:919-935`. transfer_inject's `rewrite_partition_tiles_inner` (lines 1627-1687) constructs xfer tile bounds with ALL partitioned axes regardless of whether they index the specific data symbol. wait_slice then mis-maps. Matmul is the FIRST shipped algorithm where the partitioned iv does NOT index every data symbol (b is not indexed by i, a is not indexed by j).
- FILED PREREQUISITE: TASK-0301 (HIGH) — transfer_inject + wait_slice must filter tile bounds by data access pattern. The data needed is already on `DataflowEdge::data_in_access` (per transfer_inject docs); the filter is one upstream pass away.

CURRENT STATE:
- TASK-0297 marked BLOCKED on TASK-0301 (depends_on added).
- Draft schedule kept in place with explicit BLOCKED header pointing to TASK-0301.
- 4 [[skip]] entries added to nuc-nucleus/e2e-matrix.toml citing TASK-0301 (so the harness reports SKIPPED instead of FAIL/diff noise). e2e baseline 100/84/0/16/0 → 104/84/0/20/0 (+4 skip cells).
- When TASK-0301 lands, change [[skip]] → [[required]] in matrix, verify bit-identical, close TASK-0297.

DECISION RATIONALE (cycle 117):
- Considered partition=workers (1D) vs partition=blocks2d (2D). Both hit the same limit; blocks2d hits it on BOTH a and b; workers hits it only on b. partition=workers is the simpler exerciser of the gap.
- Considered honest-scope-down to "single-worker matmul + per-worker broadcast of full b" — but that's a degenerate distributed shape that doesn't really stress M5 machinery. Worse than honest-blocked.
- The HIGH-priority TASK-0301 unblocks not just matmul but ANY future algorithm where a transferred data symbol is not indexed by every partitioned iv. The class of unblocked algorithms is substantial (any partial-reduction, any non-stencil distributed shape).

GOTCHAS + FORWARD-CARRY:
- Cycle 116 closed mp-tcp-bufsync's slice-paste silent-sibling. Cycle 117 closed mp-tcp-bufsync's silent-sibling AND surfaced the AXIS-MAPPING limit (a separate class of silent-corruption-bypass that the cycle-116 fix made universal across backends). Both are different shapes of "honest-partial assumption hardens silently across backends" — the architectural memory `feedback-silent-sibling-defect.md` (cycle-116 update) covers the first; this one motivates extending the "AXIS-MAPPING ASSUMPTION" doc reference into the same memory file when TASK-0301 lands.
- Verify after TASK-0301 lands: 05/distributed + 05/distributed-2d + 06/distributed cells remain byte-identical (the fix must not regress cases where every partitioned iv DOES index every data symbol).
<!-- SECTION:NOTES:END -->
