---
id: TASK-0259
title: >-
  M5 sub-task: partition=blocks2d consumer pass (2D-block partitioning of a 2D
  loop nest)
status: To Do
assignee: []
created_date: '2026-05-23 23:53'
labels:
  - M5
  - compiler
  - partition
dependencies:
  - TASK-0043
  - TASK-0249
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD §6.3.3 + TASK-0043 AC#1. partition=blocks2d is currently REJECTED at sched-lower as UnsupportedPartitionKind (TASK-0249 cycle 70). M5 needs a real consumer.

## Scope
Add nucleus/nucleus-compiler/src/passes/partition_blocks2d.rs as a sibling to passes/partition_workers.rs and the partition=rows consumer. Walks the ACFG, finds nested Repeat nodes with ResolvedLoopOption::Partition(PartitionKind::Blocks2d) on the outer of a 2D pair, and partitions BOTH iteration ranges across a 2D grid of workers (W_rows × W_cols = N workers from the place-set).

## Acceptance Criteria
1. partition_blocks2d pass exists; called from passes/mod.rs.
2. A 2D Repeat-of-Repeat with partition=blocks2d on a place-set sized as a 2D grid produces per-worker (row_band × col_band) ranges in NameSidecar.partition_worker_ranges.
3. partition=blocks2d on a non-2D nest is rejected at sched-lower as typed UnsupportedPartitionKind.
4. The non-2D-nest reject precedence (TASK-0144.02 envisioned) merges with this task — file a CLOSURE note on TASK-0144.02 when this sub-task lands.
5. The UnsupportedPartitionKind reject for Blocks2d is REMOVED from sched-lower when this consumer lands.
6. A new e2e cell exercises partition=blocks2d on example 7 (matmul) or 5 (stencil — 2D); bit-identical vs reference.bin on at least one tier-1 backend.

## Open questions
- Grid-shape inference from the worker count: SQRT-and-round, factor decomposition, or schedule-explicit (partition=blocks2d(R,C))? Default: factor decomposition with deterministic tiebreaker (largest-square if N is a perfect square; otherwise the factor pair closest to square).
<!-- SECTION:DESCRIPTION:END -->
