---
id: TASK-0306
title: >-
  transfer_inject + inject_halo_strip_xfers: data-layout / iv-permutation-aware
  bounds emission (TASK-0302 cycle-121 architect P2.1)
status: To Do
assignee: []
created_date: '2026-05-25 03:55'
updated_date: '2026-05-25 05:27'
labels:
  - M6
  - compiler
  - transfer_inject
  - axis-mapping
  - halo-strip
  - forward-carried-from-TASK-0302
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0302 (cycle 121) closed the data-dim contiguous-prefix axis-mapping gap via `compute_partition_bounds_with_dim_prefix`. The architect's review-gate flagged two latent shapes that the new logic does NOT cover but which are absent from every shipped schedule today:

1. **Halo-strip x "non-prefix" data layout**: `inject_halo_strip_xfers` at `nucleus/nucleus-compiler/src/passes/transfer_inject.rs:2721-2773` writes `[(outer_iv, ...), (inner_iv, ...)]` ASSUMING the data is `[outer][inner]`. For a halo-bearing data symbol indexed `[k][j]` while the partition pair is `(outer=i, inner=j)` the halo-strip bounds would mis-map. The halo-strip site does NOT yet consult `data_dim_iv_map`.

2. **Inner-axis-leading partition**: e.g. `partition=blocks2d(j, i)` where the OUTER iv lands at data dim 1 instead of 0. The dim-prefix logic assumes dim 0 comes first.

## Why not blocking TASK-0302

Both shapes are LATENT. Every shipped M5 cell + the new 07-matmul/distributed-2d cell uses `[outer][inner]` data layout AND outer-axis-leading partition. 05-stencil's stencil halo-bearing data is indexed in nest order. 07-matmul's halo-bearing kernel is absent (madd has zero halo).

## Acceptance criteria

1. `inject_halo_strip_xfers` consults `data_dim_iv_map` (or equivalent per-data dim info) when constructing halo-strip tile bounds — for a halo-bearing data indexed `[k][j]` with partition pair `(outer=i, inner=j)`, emit either dim-correct bounds OR drop to whole-array.
2. The dim-prefix logic in `compute_partition_bounds_with_dim_prefix` is extended to support an inner-axis-leading partition (data dim of outer iv > data dim of inner iv): emit in dim order regardless of partition nest order.
3. A pinning test that constructs a synthetic halo-bearing data with non-outer-leading dim layout and asserts the halo-strip bounds are dim-correct.
4. A pinning test for the inner-axis-leading partition shape.
5. Existing M5 cells (05/distributed, 05/distributed-2d, 06/distributed, 07/distributed, 07/distributed-2d) preserved byte-identical.

## Cross-references

- `nucleus/nucleus-compiler/src/passes/transfer_inject.rs:2721-2773` — `inject_halo_strip_xfers` site.
- `nucleus/nucleus-compiler/src/passes/transfer_inject.rs:compute_partition_bounds_with_dim_prefix` — extends here.
- `nucleus/backend-common/src/multi_worker_walker.rs:919-955` — AXIS-MAPPING ASSUMPTION doc with the open-shapes paragraph TASK-0302 added.
- TASK-0302 cycle 121 architect P2.1.

## Honest scope

LOW priority. Not a current regression; defends against a future M6+ schedule that constructs either shape.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0310 cycle 125: when adding the iv-permutation-aware behaviour-pin tests AC#3 + AC#4, be aware that under partition=blocks2d transfer_inject emits TWO Push classes for each halo-bearing data: (a) host-broadcast main Pushes carrying band±halo on each axis, (b) inject_halo_strip_xfers cross-worker halo strips carrying 1-row / 1-column slices. A naive XferRole::Push && data == X_id filter catches BOTH. The cycle-125 task0310_05 idiom disambiguates by checking x.src ∈ partition_worker_ranges[outer_iv].keys() (compute worker → strip) vs not (host → main). Discovered empirically by a real test failure on cycle-125 first run; pure narrative would have missed it. See nucleus/nucleus-compiler/tests/sidecar_halo.rs:task0310_05_* for the canonical filter idiom. Also: any test name with uppercase letters (the literal _y_AND_x suffix the TASK-0310 brief proposed) fails clippy -D non_snake_case; use _y_and_x instead.
<!-- SECTION:NOTES:END -->
