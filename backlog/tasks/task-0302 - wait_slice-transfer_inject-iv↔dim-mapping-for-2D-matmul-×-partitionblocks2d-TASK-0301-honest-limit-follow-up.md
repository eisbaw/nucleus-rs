---
id: TASK-0302
title: >-
  wait_slice + transfer_inject: iv↔dim mapping for 2D matmul ×
  partition=blocks2d (TASK-0301 honest-limit follow-up)
status: To Do
assignee: []
created_date: '2026-05-25 02:10'
labels:
  - M5
  - compiler
  - transfer_inject
  - wait_slice
  - axis-mapping
  - 2d-matmul
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background
TASK-0301 (cycle 118) closed the 1D AXIS-MAPPING ASSUMPTION case by adding a per-data iv filter to `rewrite_partition_tiles_inner`. The filter excludes from a data symbol's tile bounds any iv that is NOT observed in any of the symbol's access expressions (via DataflowEdge::data_in_access / data_out_access).

That filter is sufficient for 07-matmul × partition=workers(i): `b` is indexed [k][j], so the filter excludes i from b's bounds → empty bounds → whole-array broadcast of b → bit-identical compute.

## Concrete limit this task surfaces
For a hypothetical 07-matmul × partition=blocks2d(i, j), the filter would produce `b` bounds = [(j, j_band)] (j IS in b's observed iv set). But wait_slice's axis-mapping convention still presumes `bounds[i].iter_var` indexes data dim i. b's dim 0 is k (not j); slicing `b[0..j_band.end][full]` by j_band would silently mis-address b's k axis.

The minimum extension: `wait_slice` needs an iv → data-dim mapping (per data symbol) and must select bounds entries by mapping each iv to the dim it actually indexes, rather than relying on the nest-order convention.

## Acceptance criteria
1. Either `wait_slice` accepts a per-(data, iv → dim) mapping built from the same DataflowEdge::data_in_access carry that TASK-0301 already consumes; OR `rewrite_partition_tiles_inner` builds the bounds in dim-order keyed by which dim each iv indexes (so wait_slice's existing convention keeps working).
2. A 07-matmul × partition=blocks2d(i, j) schedule (filed as part of this task) lowers bit-identical on at least one tier-1 backend.
3. Existing M5 cells (05/distributed, 05/distributed-2d, 06/distributed, 07-matmul/distributed) byte-identical preserved.
4. The cross-axis iv ↔ dim invariant gets a pinning test that fails LOUD if a hypothetical `b[k][j]` × partition=blocks2d(i,j) schedule mis-maps j_band onto k's slice.

## Cross-references
- TASK-0301 (cycle 118) — landed the per-data filter at `nucleus/nucleus-compiler/src/passes/transfer_inject.rs::rewrite_partition_tiles_inner` (search for `data_iv_indexing`). The filter ALREADY has the access info needed; what's missing is the iv-to-dim mapping (currently still presumed nest-order).
- nucleus/backend-common/src/multi_worker_walker.rs:919-935 — AXIS-MAPPING ASSUMPTION doc; update when the iv→dim mapping ships.

## Honest scope
- Today no shipped schedule constructs this shape (07-matmul/distributed uses partition=workers on i alone, not blocks2d). So the limit is a *latent* shape problem, not a regression.
- The filter prevents the wrong-axis slice in the partition=workers case (the immediate matmul shape) because it produces empty bounds for b → wait_slice's empty-tile arm returns None → whole-array. For partition=blocks2d, the filter produces non-empty-but-misordered bounds, which the existing wait_slice can't disambiguate without the mapping.

## Forward-carry from TASK-0301 cycle 118
The filter helper `collect_data_iv_indexing` in transfer_inject.rs records iv UNION per data symbol; for the dim mapping we need PER-AXIS info: which iv indexes data.dim[0], which indexes dim[1], etc. Walking the same DataAccess.indices in axis order (first index → dim 0, etc.) is the extension.
<!-- SECTION:DESCRIPTION:END -->
