---
id: TASK-0216
title: >-
  partition=workers + pipeline=D coverage; partition-rewrite path builds tile
  bounds in IterVar id order not nest order
status: To Do
assignee: []
created_date: '2026-05-21 14:10'
labels:
  - compiler
  - partition
  - M4
  - latent
dependencies:
  - TASK-0134
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architecture-review finding (TASK-0134 cycle): rewrite_partition_tiles_inner at transfer_inject.rs:1583+ builds IterTile bounds by iterating partition_ranges (BTreeMap<IterVar, ...>) in IterVar-id order, NOT nest order. For typical schedules id-order coincides with nest-order (IterVar IDs are walk-order assigned), but it is not guaranteed by the IterTile::bounds convention 'outer-most first'. The 'innermost wins' semantic in annotate_pipeline_depth_for_seq's .rev() walk silently breaks when nest-order != id-order. Combining partition=workers with pipeline=D is also not exercised by any test or example.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Synthetic fixture (or example-13 batch_parallel + pipeline=) that combines partition=workers with pipeline=D on the partitioned loop. Assert pipeline_depth_for_seq is populated correctly for each per-worker fan-out pair.
- [ ] #2 Fix rewrite_partition_tiles_inner: build bounds in nest-order, not IterVar-id order. The fix is to walk the enclosing Repeat stack instead of partition_ranges. Or: assert at construction that the produced ordering matches the existing tile's ordering.
- [ ] #3 Add forward-carry into TASK-0042.01 (pthreads-async): the codegen ring-buffer pre-fill must apply per fan-out (src,dst) pair, not per data symbol — one initial_marking entry per (data, src_worker, dst_worker) tuple.
<!-- AC:END -->
