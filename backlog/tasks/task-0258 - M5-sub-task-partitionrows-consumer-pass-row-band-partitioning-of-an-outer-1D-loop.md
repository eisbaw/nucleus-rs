---
id: TASK-0258
title: >-
  M5 sub-task: partition=rows consumer pass (row-band partitioning of an outer
  1D loop)
status: To Do
assignee:
  - '@mped-architect-impl'
created_date: '2026-05-23 23:53'
updated_date: '2026-05-23 23:56'
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
DESCRIPTION CORRECTION + clarification (orchestrator cycle 79b, pre-implementer): the original description said 'row-band partitioning of an outer 1D loop'. PRD §6.3.3 line 519 is explicit: 'partition=rows on a 1D iteration' is a BAD COMBINATION rejected at compile time. partition=rows is specifically for the OUTER of a 2D nest — it row-bands the outer (y) loop, leaving the inner (x) loop intact per worker. This is the original 05-stencil/distributed use case TASK-0249 surfaced (the inert  directive on a 2D y/x nest).

Refined scope for the implementer:
1. partition_rows pass applies ONLY when the partition=rows directive is on the OUTER loop of a 2D nest (Repeat-of-Repeat in the ACFG, on the same worker entity). Reject otherwise at sched-lower OR at the pass entry, with a typed UnsupportedPartitionKindFor1DLoop variant (NOT the existing UnsupportedPartitionKind blanket reject — that becomes too coarse).
2. Semantics: row-band the outer-loop range across the placement workers (same algorithm partition_workers uses for 1D, but applied to the outer of the 2D); inner loop body executes unchanged per worker.
3. Output: NameSidecar.partition_worker_ranges[outer_iv][worker_id] = row_band_range, exactly as partition_workers populates today. No NEW sidecar field needed — transfer_inject + the backend walker already consume partition_worker_ranges and apply per-worker slice handling (host-side gather via render_wait_assign).
4. The reject site at sched/lower.rs::lower_loop_option's PartitionKind::Rows arm: REMOVE the UnsupportedPartitionKind reject for Rows (keep for Blocks2d until TASK-0259 lands). Replace with an accept-and-route-to-consumer arm.
5. The NEW reject site (typed UnsupportedPartitionKindFor1DLoop or similar) fires when partition=rows is applied to a non-outer-of-2D context. Test both negative paths.

This is mostly a 'wire partition=rows through the existing partition_workers infra' task; the heavy lifting (per-worker range -> sidecar -> emit) already exists. Estimated scope: ~150-250 LoC including tests, mostly mechanical.

Halo inference (TASK-0260) is a SIBLING task — partition_rows alone does NOT solve the stencil halo problem; without halo widths, a row-band-partitioned stencil produces wrong output at the band boundaries. Plan ahead: when this task lands and an e2e cell is added, ensure either (a) the cell's algorithm has no halo (the cell verifies partition=rows mechanism only), or (b) the cell SKIPS until TASK-0260 lands. Pure partition=rows without halo will produce incorrect output on stencils — do NOT mark the cell [[required]] until halo inference is also wired.
<!-- SECTION:NOTES:END -->
