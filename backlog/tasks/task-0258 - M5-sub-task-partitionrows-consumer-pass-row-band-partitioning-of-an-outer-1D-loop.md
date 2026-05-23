---
id: TASK-0258
title: >-
  M5 sub-task: partition=rows consumer pass (row-band partitioning of an outer
  1D loop)
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
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD §6.3.3 + TASK-0043 AC#1. partition=rows is currently REJECTED at sched-lower as UnsupportedPartitionKind (TASK-0249 cycle 70). M5 needs a real consumer.

## Scope
Add nucleus/nucleus-compiler/src/passes/partition_rows.rs as a sibling to passes/partition_workers.rs. Walks the ACFG, finds Repeat nodes with ResolvedLoopOption::Partition(PartitionKind::Rows), and partitions the OUTER iteration range across the placement's worker set (round-robin band assignment by default).

## Acceptance Criteria
1. partition_rows pass exists; called from passes/mod.rs in the canonical pass order.
2. A 1D outer Repeat with partition=rows on a place-set of N workers gets per-worker row-band ranges in NameSidecar.partition_worker_ranges (same shape partition=workers uses today).
3. partition=rows on a NON-1D loop is rejected at sched-lower as a typed error (UnsupportedPartitionKind or a new variant — matches PRD §6.3.3 'bad combinations rejected at compile time').
4. UnsupportedPartitionKind for Rows is REMOVED from sched-lower (TASK-0249 reject becomes accept-and-route-to-consumer).
5. A new e2e cell exercises partition=rows on examples 5 or 6; bit-identical vs reference.bin on at least one tier-1 backend.

## Open questions
- Round-robin row-band vs strict equal-band assignment for non-divisible row counts. Default: same trailing-partial discipline that block_transform.rs uses (TASK-0218 / TASK-0181).
- Halo inference for stencil examples (5, 6) is TASK-0043 AC#2 — sibling task, not this one.

## Forward-carry from TASK-0249
The reject site at sched/lower.rs::lower_loop_option (the PartitionKind::Rows arm of UnsupportedPartitionKind) must be REMOVED when this consumer lands; otherwise the schedule never reaches the partition_rows pass. Same surgical edit pattern partition_workers used when it landed.
<!-- SECTION:DESCRIPTION:END -->
