---
id: TASK-0301
title: >-
  transfer_inject + wait_slice: per-data tile bounds (filter by data access
  pattern) — unblock matmul/distributed
status: Done
assignee:
  - '@mped-orchestrator'
created_date: '2026-05-25 01:29'
updated_date: '2026-05-25 02:23'
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

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
ORCHESTRATOR-DIRECT IMPLEMENTATION (per memory: spawned-agents-refuse-code-edits).

APPROACH: Option (1) from the brief — filter at xfer construction by data access pattern.

Plan:
1. Add helper in transfer_inject.rs that walks ACFG Operations to build
   data_iv_indexing: BTreeMap<DataId, BTreeSet<IterVar>>
   - For each Op's DataflowDag, scan data_in_access + data_out_access
   - Walk each DataAccess.indices IrExpr collecting Ident(name) refs
   - Resolve via name_iter_vars → IterVar
2. Pass this map to rewrite_partition_tiles + rewrite_partition_tiles_inner
3. In rewrite_partition_tiles_inner, when constructing bounds for each Xfer:
   - skip iv from partition_axis_order if NOT in data_iv_indexing[x.data]
   - empty bounds → leave tile unchanged (whole-array path, exactly what
     wait_slice's leading-tile-None arm wants)
4. Verify 05/distributed, 05/distributed-2d, 06/distributed byte-identical
   (these have every data indexed by every partitioned iv, so filter is a no-op).
5. Verify 07-matmul/distributed bit-identical on pthreads-sync (the schedule
   uses partition=workers on i; b is indexed [k][j], so filter → empty bounds
   → broadcast full b → CORRECT compute on every worker).
6. Promote [[skip]] → [[required]] for 07-matmul/distributed in e2e-matrix.toml.
7. Close TASK-0297 (whose blocker is THIS task).

HONEST-SCOPE CAVEAT (forward-carry to a new follow-up):
- Filter alone is sufficient WHEN bounds[i] axis-id ↔ data dim i holds for
  the surviving axes. For 07-matmul/distributed × partition=workers(i) it does.
- For a hypothetical 2D matmul × partition=blocks2d(i,j), b after filter
  yields bounds=[(j, j_band)] but b's dim 0 is k — wait_slice would mis-map
  silently. File as a follow-up task pre-emptively (data_iv_indexing carries
  enough info to detect; the fix is iv→dim mapping in wait_slice).

GATE: nix develop --command bash -c "just build && just clippy && just test && just test-release && just e2e"
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
ORCHESTRATOR-DIRECT IMPLEMENTATION cycle 118 (2026-05-25).

SHIPPED:
- nucleus/nucleus-compiler/src/passes/transfer_inject.rs:
  * New helper trio collect_data_iv_indexing + walk_data_iv_indexing + collect_ivs_from_expr (lines ~1758-1856 post-edit).
  * Threaded data_iv_indexing: BTreeMap<DataId, BTreeSet<IterVar>> through rewrite_partition_tiles → rewrite_partition_tiles_inner.
  * Filter is ADDITIVE: only excludes an iv from a data's bounds when the data has a NON-EMPTY observed iv set that excludes the iv. Empty observed set → fall back to pre-TASK-0301 behaviour (every partitioned iv applies). This preserves synthetic test fixtures using DataflowEdge::new (which constructs empty indices) without churn, AND preserves every shipped M5 cell (where every partitioned iv IS observed in every data symbol's accesses).
- nucleus/nucleus-compiler/tests/transfer_inject.rs: added pinning test rewrite_partition_tiles_filters_non_indexing_iv_for_07_matmul_shape (architect P2.1). Pins the 07-matmul shape at unit speed: a is [i][k] → bounds = [(i, i_band)]; c is [i][j] → bounds = [(i, i_band)]; b is [k][j] → bounds = EMPTY (whole-array broadcast).
- nuc-nucleus/e2e-matrix.toml: 4 [[skip]] for 07-matmul/distributed × {pthreads-sync, mp-tcp-bufsync, pthreads-async, mp-tcp-event} promoted to [[required]] M5.
- nuc-nucleus/examples/07-matmul/schedules/distributed.sched.nuc: replaced BLOCKED draft with production schedule.

GATE: green. e2e 104/84/0/20/0 → 104/88/0/16/0 (+4 cells promoted, 0 regressions). Two e2e back-to-back runs gave identical totals (qa-test-runner verified).

REVIEW GATE (cycle 118 parallel read-only):
- qa-test-runner: GO. All gates green; e2e deterministic across 2 runs; existing M5 cells unchanged.
- mped-architect: GO. P1 = none. P2.1 (unit test) APPLIED. P3.1 (doc tightening on collect_data_iv_indexing extend_xfer_tiles_for_halo cross-reference) APPLIED. P3.3 (silent-sibling audit note at filter site) APPLIED. P2.2 + P3.4 (NUC_TRACE on fall-back arms / Call-in-index recursion) declined — trace not currently imported in transfer_inject.rs; the fall-back is fully documented and the Call arm is unreachable in today's grammar.

HONEST LIMIT (filed as TASK-0302):
- 07-matmul × partition=blocks2d(i, j) would still mis-map: filter leaves b's bounds = [(j, j_band)] but wait_slice's nest-order convention treats bounds[0] as dim 0 = k. Latent (no shipped schedule constructs this), but documented and tracked. The fix is the iv→dim mapping in wait_slice that the brief lists as Option 2.

GOTCHAS + FORWARD-CARRY:
- The additive contract is load-bearing: switching to a strict 'absent → skip every iv' shape broke 8 transfer_inject tests + 1 partition_workers test, all using DataflowEdge::new which constructs empty indices. Strict semantics is technically correct (no observed iv → no slicing) but the test-fixture cost is high. The additive variant is empirically equivalent on every shipped production cell (where access indices are always populated by build_acfg from AlgoIR) and zero-cost for synthetic tests. Future code may want a strict mode for production-only fail-loud.
- The architect's silent-sibling audit (P3.3) is cycle-118 ground truth: every other site that touches x.tile (inject_in_node_with_tile, extend_xfer_tiles_for_halo, inject_halo_strip_xfers) is either source-of-truth structural or runs after the filter and consumes the already-filtered bounds. A future N-dim halo pass that builds bounds from partition_axis_order must consult data_iv_indexing — locked in a comment at the filter site.
<!-- SECTION:NOTES:END -->
