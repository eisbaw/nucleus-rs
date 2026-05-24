---
id: TASK-0266
title: >-
  Investigate 05-stencil/distributed × pthreads-async deadlock after halo wiring
  (Stage-2 runtime gap)
status: To Do
assignee: []
created_date: '2026-05-24 04:04'
labels:
  - M5
  - bug
  - compiler
  - deadlock
  - stage-2
dependencies:
  - TASK-0262
  - TASK-0263
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

Cycle 83 landed TASK-0262 (floor-with-spillover remainder policy for partition_rows/partition_workers) and TASK-0263 (transfer_inject consumes halo_widths to extend per-tile transfer ranges). The intent was to close M5 (TASK-0043) AC#4 by promoting 05-stencil/distributed × pthreads-async to [[required]] and verifying bit-identical to reference.bin.

## Observed failure

Cell promoted to [[required]]; `just e2e` started; the generated nuc-generated binary (PID 3309558) HUNG indefinitely. No progress past 30 minutes; the e2e harness was stalled on cell 39 of 88 ("05-stencil | distributed | pthreads-async ..." — no PASS/FAIL printed). Killed manually.

The cycle-83 implementer's intermediate commits stay (624d7dc + cf2f9ac): the partitioning + halo-extended tile arithmetic are CORRECT in isolation; the runtime semantic of the synthesised halo transfers is the gap.

## Candidate root causes

1. **Circular Push/Wait dependency between adjacent workers.** Each worker w_i now Pushes its boundary rows to w_{i-1} and w_{i+1} (and Waits for theirs). If the Push/Wait ordering across the workers forms a cycle, the Condvar coordination in pthreads-async's Ring<T> deadlocks. transfer_inject's fan-out per-(src,dst) pair may have generated an ordering that interlocks.
2. **Off-by-one in tile-with-halo boundary arithmetic.** The kernel blur3 reads grid[y-1], grid[y], grid[y+1]. At y=lo_i (the bottom of worker w_i's band), grid[y-1] is in w_{i-1}'s band. The halo extension should have shipped that row, but if the per-worker partition_worker_ranges is now [lo_i - halo, hi_i + halo) (inclusive halo expanded into the read range) AND the kernel still reads grid[y-1] expecting the absolute index, the per-worker tile may already cover the read — OR the absolute-index rebinding may double-add the halo.
3. **Missing halo-strip Push at the partition seam.** transfer_inject extends the tile bounds but may not have added the Push/Wait pairs that ship the halo rows. The tile-extension changes which rows the worker READS but does not by itself synthesise the cross-worker transfer; the Push is implicit in the host-to-worker fan-out today, which assumes whole-array push (TASK-0117). For partitioned workers, the host's tile is the FULL grid, but each worker now waits for its (extended) row band — the host's transfer must include the halo rows for adjacent bands.

## What's needed

1. Reproduce the hang in a controlled fixture (smaller grid, e.g. H=4, W=4, 2 workers, halo=1).
2. Use NUC_TRACE=1 or strace on the worker threads to identify whether the hang is in Push (waiting for ring space), Wait (waiting for arrival), or compute (infinite loop in the kernel).
3. Based on the trace:
   - (a) If Push/Wait cycle: change transfer_inject's per-pair fan-out order OR introduce a topological ordering on the halo Push/Wait pairs.
   - (b) If off-by-one: inspect the emitted main.rs at `nucleus/target/e2e-matrix/run-*/05-stencil__distributed__pthreads-async/src/main.rs` and trace the per-iteration index arithmetic against the kernel signature.
   - (c) If missing halo strip: extend transfer_inject to synthesise additional cross-worker XferPlaceholders for the halo rows, not just extend the existing whole-array push's range.

## Acceptance

1. The 05-stencil/distributed × pthreads-async cell promotes from [[skip]] to [[required]] and PASSES bit-identical to reference.bin (sha256: read from examples/05-stencil/reference.bin).
2. `just e2e` total 88 / 74 pass / 0 fail / 14 skip / 0 required-fail.
3. `just determinism-check` continues to PASS (byte-identical re-emit).
4. Root cause documented + tests added that pin the fix (regression test for the deadlock; positive test for the halo-strip cross-worker Push/Wait shape).

## Dependencies
- TASK-0262 (remainder policy) — landed.
- TASK-0263 (transfer_inject halo extension) — landed.
- TASK-0260 (halo Stage 1) — landed.

## Forward-carry context
This is the closing keystone for M5 AC#4 (TASK-0043). The full M5 differential matrix on examples 5/6/7 distributed depends on this task closing. Until it lands, M5 AC#4 stays partial.
<!-- SECTION:DESCRIPTION:END -->
