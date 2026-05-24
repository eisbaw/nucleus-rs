---
id: TASK-0264
title: >-
  M5 Stage 3: block-pair metadata recovery for halo-strip synthesis under
  partition=blocks2d
status: To Do
assignee: []
created_date: '2026-05-24 01:40'
labels:
  - M5
  - compiler
  - halo
  - partition
  - stage-3
dependencies:
  - TASK-0260
  - TASK-0259
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Stage 3 of the TASK-0260 halo loop. Stage 1 (TASK-0260, cycle 81) landed halo inference; Stage 2 (TASK-0263) will wire transfer_inject. This task addresses the TASK-0259 architect forward-carry: halo-strip Push/Wait synthesis under partition=blocks2d needs to identify the (row, col) neighbours of each worker.

## Problem
partition_blocks2d (TASK-0259) writes TWO entries into ACFG::partition_worker_ranges (one per iter_var, same WorkerId keyset) but does NOT carry block-pair metadata. A future halo-strip synthesis stage cannot tell from the sidecar alone whether two iter_var->Range maps come from one partition=blocks2d directive (paired axes; w_i owns (y_i, x_i) rectangle) OR from two independent partition=rows directives on unrelated loops.

## Acceptance criteria
1. Either re-derive pairing by walking linked.sched.loops for PartitionKind::Blocks2d directives, OR add ACFG.partition_pairs: BTreeMap<IterVar, IterVar> populated by partition_blocks2d. Pick consciously.
2. Worker -> (row, col) inverse: expose partition_blocks2d::decompose_grid as pub(crate), or add sidecar.grid_shape_for_outer_iv: BTreeMap<IterVar, (u32, u32)>. Pick consciously.
3. Halo-strip Push/Wait synthesis identifies the correct neighbours (N/S/E/W cells in 2D grid) for each worker under partition=blocks2d.
4. New e2e cell 05-stencil/distributed-2d x pthreads-async bit-identical to reference.bin.

## References
- TASK-0259 partition_blocks2d implementation: nucleus/nucleus-compiler/src/passes/partition_blocks2d.rs
- TASK-0259 architect forward-carry notes: backlog task view TASK-0259
<!-- SECTION:DESCRIPTION:END -->
