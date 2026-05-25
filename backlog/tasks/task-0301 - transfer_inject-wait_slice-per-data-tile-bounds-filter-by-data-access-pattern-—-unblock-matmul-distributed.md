---
id: TASK-0301
title: >-
  transfer_inject + wait_slice: per-data tile bounds (filter by data access
  pattern) — unblock matmul/distributed
status: To Do
assignee: []
created_date: '2026-05-25 01:29'
labels:
  - M5
  - compiler
  - transfer_inject
  - wait_slice
  - backend-common
  - axis-mapping
  - forward-carried-from-TASK-0297
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background
TASK-0297 cycle 117 attempted to ship 07-matmul/distributed. Empirically confirmed: the AXIS-MAPPING ASSUMPTION limit (documented in `nucleus/backend-common/src/multi_worker_walker.rs:919-935`) silently corrupts the emit for any algorithm where the partitioned iv does NOT index a data symbol that gets transferred.

## Concrete failure observed
07-matmul/prog.algo.nuc:
```
for i : 0..N { for j : 0..N { for k : 0..N {
    c[i][j] <-- madd(c[i][j], a[i][k], b[k][j]);
}}}
```

With `loop i : partition=workers` and 4 workers, each worker gets `i_band = N/4 = 4` rows.

`transfer_inject::rewrite_partition_tiles_inner` sets EVERY xfer tile to `bounds = [(i, i_band_for_w)]` regardless of which axes index the data. Then `wait_slice` treats `tile.bounds[0]` as a slice of `data.dim[0]`. Result on worker 0:
- a: `a[0..64].copy_from_slice(&_tmp[0..64])` — CORRECT (a is a[i][k], i = leading axis, sliced by i_band).
- b: `b[0..64].copy_from_slice(&_tmp[0..64])` — WRONG. b is b[k][j], leading dim is k. i_band silently slices k axis. Worker 0 gets b[k=0..4][full j] instead of b[full k][full j], so its compute reads zero-default for k=4..16.
- c: `c[0..64]...` — CORRECT on the gather (c is c[i][j], same shape as a).

Bit-identical fails because b is mis-sliced.

## Acceptance criteria
1. EITHER `transfer_inject::rewrite_partition_tiles_inner` constructs per-data tile bounds — filtering `partition_axis_order` to include only axes that index the specific data symbol on this xfer placeholder; OR
2. `wait_slice` takes a `(data_id, iv_axis_for_data)` mapping and uses only the axes that align between tile and data; OR
3. A new sidecar field `data_iv_indexing: BTreeMap<DataId, BTreeSet<IterVar>>` populated by an upstream pass and consulted by transfer_inject + wait_slice to filter axes.
4. Whichever shape: 07-matmul/distributed.sched.nuc lowers bit-identical against reference.bin on at least one tier-1 backend.
5. Existing M5 cells (05/distributed, 05/distributed-2d, 06/distributed) byte-identical preserved — the fix must not alter emit for cases where every tile axis already aligns with every data symbol.

## Dependencies
- Unblocks: TASK-0297 (07-matmul/distributed M5 closeout, AC#4 example 7).

## Cross-references
- `nucleus/backend-common/src/multi_worker_walker.rs:919-935` — the AXIS-MAPPING ASSUMPTION doc that warns about exactly this case.
- `nucleus/nucleus-compiler/src/passes/transfer_inject.rs:1627-1687` — `rewrite_partition_tiles_inner` that constructs all-axes tiles.
- `nucleus/nucleus-compiler/src/acfg.rs` — `DataflowEdge::data_in_access` / `data_out_access` already carry per-firing index expressions (per transfer_inject docs line 119-128); the data needed for the filter is there, just not currently consulted.

## Honest scope
- HIGH priority because it gates the M5 AC#4 example 7 closeout.
- 1-2 cycles when picked up. Option (1) (filter at xfer construction) seems simplest — the access patterns are reachable via the existing `data_in_access` ACFG carry; the filter is a structural step.

## Forward-carry from TASK-0297 cycle 117
- 07-matmul/distributed.sched.nuc was left as an EXPLORATORY DRAFT in the examples dir (NOT added to e2e-matrix) with a header comment pointing to this task. When this task lands, the draft becomes the production cell.
<!-- SECTION:DESCRIPTION:END -->
