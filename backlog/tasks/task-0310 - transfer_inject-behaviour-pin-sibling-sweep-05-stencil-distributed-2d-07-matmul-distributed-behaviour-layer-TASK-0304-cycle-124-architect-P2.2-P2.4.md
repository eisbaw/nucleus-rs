---
id: TASK-0310
title: >-
  transfer_inject behaviour pin sibling-sweep: 05-stencil/distributed-2d +
  07-matmul/distributed behaviour layer (TASK-0304 cycle-124 architect P2.2 +
  P2.4)
status: To Do
assignee: []
created_date: '2026-05-25 05:04'
labels:
  - M5
  - test-coverage
  - transfer_inject
  - sibling-sweep
  - forward-carried-from-TASK-0304
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0304 cycle 124 LANDED behaviour-layer pins for 05-stencil/distributed (halo=1 extension by ±1) and 06-separable-filter/distributed (halo=0 no-extension). The cycle-124 architect review-gate flagged two STRUCTURALLY IDENTICAL sibling narratives that remain unpinned at the behaviour layer — the precise pattern `feedback-silent-sibling-defect` warns about.

## What to pin

### Sibling 1: 05-stencil/distributed-2d.sched.nuc (2D-blocks2d shape, P2.2)

The 2D distributed-2d schedule for 05-stencil makes the SAME load-bearing TASK-0263 transfer_inject claim as 05/distributed but for partition=blocks2d. `task0303_05` (cycle 120) pinned the halo_widths VALUE layer for blur3 in this schedule; the BEHAVIOUR layer (per-tile transfer ranges actually extended by ±1 on both y AND x) is unpinned. A regression in transfer_inject's 2D extension path (e.g. an iv↔dim mapping defect, see open TASK-0302) would pass `task0303_05` AND `task0304_05_stencil_distributed_*` and be caught only at e2e bytes.

### Sibling 2: 07-matmul/distributed.sched.nuc (1D shape, halo=0, P2.4)

TASK-0303 (cycle 120) pinned `task0303_07_matmul_distributed_halo_widths_pinned_to_zero` (the halo_widths VALUE side: `madd_i == 0`). The BEHAVIOUR side is the SAME pattern as 06-separable-filter/distributed: halo=0 → transfer_inject does NOT extend per-tile transfer ranges. The schedule header at `nuc-nucleus/examples/07-matmul/schedules/distributed.sched.nuc:25` carries this claim (`no halo, no cross-worker carry`). Unpinned at the behaviour layer.

## Acceptance criteria

1. Add `task0309_05_stencil_distributed_2d_transfer_inject_halo_one_extension_on_img_in_y_AND_x` to `nucleus/nucleus-compiler/tests/sidecar_halo.rs`. For each img_in Push to a compute worker under partition=blocks2d, assert tile.bounds[y] AND tile.bounds[x] are EACH band±1 (or band±halo where applicable). Use the existing `lower()` helper.
2. Add `task0309_07_matmul_distributed_transfer_inject_no_halo_extension_on_a_i` to the same file. For each a Push to a compute worker under partition=workers (or whatever partition shape 07-matmul/distributed uses for i), assert tile.bounds[i] == partition band (no extension because halo_widths[madd][i] = 0).
3. Each test cites the schedule-header line range it defends and names the failure mode in the assert message (matching the task0304_* idiom).
4. e2e baseline 108/92/0/16/0 preserved.

## Implementer hint

- The TASK-0304 cycle-124 lower() helper at sidecar_halo.rs:46-68 runs the full pipeline and returns post-inject_transfers ACFG. Use `acfg.root.collect_xfers()` + filter on `XferRole::Push && data == DataId`.
- For 05/distributed-2d: read `acfg.partition_blocks2d_ranges[(y_iv, x_iv)]` for the per-worker 2D band map. The pre-cycle-124 architect noted that the 2D iv→dim mapping has known limits (TASK-0302 open) — be defensive about whether bounds carry both ivs.
- For 07/distributed: structurally identical to task0304_06_*; copy the idiom.

## Honest scope

LOW priority. The behaviour-layer regression risk for halo-bearing distributed schedules is low (the cycle-83 TASK-0263 + cycle-118 TASK-0301 + cycle-121 TASK-0302 axis-mapping passes all have extensive coverage). This task closes the across-schedule sibling-sweep gap.

## Cross-references

- TASK-0304 cycle 124 architect P2.2 + P2.4 — the gap-discovery review-gate.
- TASK-0303 cycle 120 — VALUE-layer sibling pins for 05-distributed-2d + 07-matmul/distributed.
- TASK-0302 — open 2D iv↔dim mapping limit; may bear on the 05/distributed-2d test fixture shape.
- Memory: `feedback-silent-sibling-defect` — the recurrence pattern this task closes.
<!-- SECTION:DESCRIPTION:END -->
