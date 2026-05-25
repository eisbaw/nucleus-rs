---
id: TASK-0302
title: >-
  wait_slice + transfer_inject: iv↔dim mapping for 2D matmul ×
  partition=blocks2d (TASK-0301 honest-limit follow-up)
status: Done
assignee:
  - '@mark'
created_date: '2026-05-25 02:10'
updated_date: '2026-05-25 03:57'
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

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
ORCHESTRATOR-DIRECT IMPLEMENTATION (per memory: spawned-agents-refuse-code-edits).

APPROACH (Option B refined): generalise TASK-0301's iv-membership filter into a per-dim mapping + contiguous-prefix check. The dim-prefix shape is what wait_slice's nest-order convention requires; sparse coverage (e.g., b[k][j] under partition=blocks2d(i,j) where j indexes dim 1 but no partitioned iv indexes dim 0) falls back to whole-array broadcast (exactly the same shape TASK-0301 already uses for the i-filter case).

Steps:
1. Extend transfer_inject.rs with collect_data_dim_iv_map + walk_data_dim_iv_map (sibling to collect_data_iv_indexing). Produces BTreeMap<DataId, Vec<Option<IterVar>>> indexed by dim — extracts the leaf IterVar from each DataAccess.indices expr; multi-access disagreement collapses to None.
2. Add helper compute_partition_bounds_for_data(data_id, data_dims, dim_iv_map, partition_axis_order, partition_ranges, worker) → Vec<(IterVar, Range<i64>)>. For each dim of the data: look up iv, check partitioned + has range for worker. Check covered dims form a contiguous prefix from dim 0. If yes: emit bounds in dim order. If no: return empty (whole-array).
3. Replace the existing loop+filter at rewrite_partition_tiles_inner with a call to the helper, passing data dims from sidecar.data_type(x.data).dims.
4. Update collect_data_iv_indexing's doc + call sites (the per-dim map subsumes the per-symbol union; the additive contract semantics shift from "membership filter" to "dim-prefix filter", which is observationally equivalent on every shipped cell where partitioned axes ARE a prefix).
5. Verify shipped cells unchanged:
   - 05/distributed (workers on y, data [y][x]): prefix {0}, identity.
   - 05/distributed-2d (blocks2d on y/x, data [y][x]): prefix {0,1}, identity.
   - 06/distributed (workers on y, data [y][x]): prefix {0}, identity.
   - 07/distributed (workers on i): a[i][k] prefix {0}, c[i][j] prefix {0}, b[k][j] non-prefix dim {} (i doesn't index b → no coverage) → empty bounds, identity with pre-fix.
6. Author 07-matmul/schedules/distributed-2d.sched.nuc + e2e cell on pthreads-sync.
7. Verify bit-identical (existing reference.bin applies — partition shape doesn't change c's k-fold).
8. Add pinning test that constructs a synthetic data with dim {1} indexed by j only (no iv for dim 0), partitions j, asserts bounds = [] (non-prefix → whole-array). Pinning fires LOUD if a future change re-imports the silent mis-mapping.
9. Update AXIS-MAPPING ASSUMPTION doc at multi_worker_walker.rs:919-935 — replace HONEST-PARTIAL note with TASK-0302 discharge.

GATE: nix develop --command bash -c "just build && just clippy && just test && just test-release && just e2e"

GOTCHAS TO WATCH:
- The per-dim Option<IterVar> needs careful aggregation across multiple accesses on the same data (read + write, two reads). Conservative: if any access has a complex expr at dim i, or two accesses disagree on the iv at dim i, treat as None.
- DataflowEdge::new (synthetic test fixture) constructs empty indices — same fall-back contract as TASK-0301: empty per-dim map → behave like pre-existing 'every partitioned iv applies' (since per-dim coverage check sees no dims to compare).
- The change SUBSUMES TASK-0301's union filter (an iv that indexes NO dim of the data contributes to no covered dims, falls into the non-prefix branch when other axes ARE partitioned, OR empty when it's the only axis). Verify TASK-0301's pinning test (rewrite_partition_tiles_filters_non_indexing_iv_for_07_matmul_shape) still passes.
- Bit-identical guarantee: under partition=blocks2d(i,j) for 07-matmul, c[i][j] = sum_k a[i][k] * b[k][j] is computed identically per-worker (each worker's c_band is the same regardless of which worker computes it). reference.bin is invariant. Cross-check on pthreads-sync first.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
ORCHESTRATOR-DIRECT IMPLEMENTATION cycle 121 (2026-05-25).

SHIPPED:
- nucleus/nucleus-compiler/src/passes/transfer_inject.rs:
  * REPLACED collect_data_iv_indexing + walk_data_iv_indexing with the per-dim sibling pair: collect_data_dim_iv_map + walk_data_dim_iv_map + record_access_per_dim (Vec<BTreeSet<IterVar>> per data, indexed by dim position via DataAccess.indices Vec position).
  * NEW compute_partition_bounds_with_dim_prefix helper — returns Some(bounds) in DATA-DIM order when coverage forms a contiguous prefix from dim 0; Some(empty) when sparse OR ambiguous (multi partitioned iv per dim); None when data has no observed indexed accesses (fall-back to pre-TASK-0301 nest-order iteration).
  * Updated rewrite_partition_tiles + rewrite_partition_tiles_inner signatures + filter body to call the new helper. partition_axis_order is now consulted only on the None (fall-back) path.
- nuc-nucleus/examples/07-matmul/schedules/distributed-2d.sched.nuc — new, partition=blocks2d on (i, j).
- nuc-nucleus/e2e-matrix.toml — 4 new [[required]] cells for 07-matmul/distributed-2d on all 4 tier-1 backends.
- nucleus/nucleus-compiler/tests/transfer_inject.rs — 2 new pinning tests:
  * rewrite_partition_tiles_dim_prefix_check_for_07_matmul_blocks2d_shape: pins the 07-matmul/blocks2d shape at unit speed (a dim {0} prefix; c dim {0,1} prefix; b dim {1} non-prefix → empty bounds).
  * rewrite_partition_tiles_drops_ambiguous_multi_partitioned_iv_per_dim (architect P2.2): pins the ambiguity arm using IrExpr::BinOp(IrBinOp::Add, ident('i'), ident('j')) on a dim, both partitioned → empty bounds.
- nucleus/backend-common/src/multi_worker_walker.rs — replaced AXIS-MAPPING ASSUMPTION doc with TASK-0302 discharge + lineage (TASK-0117 → 0294 → 0301 → 0302) + an explicit 'Open shapes (not in e2e matrix)' paragraph (architect P3.1 wording softened to acknowledge that inject_halo_strip_xfers does not yet consult data_dim_iv_map and the inner-axis-leading partition is still latent).

GATE: green. e2e 108/92/0/16/0 confirmed non-flake across 4+ independent runs (qa-test-runner verified 3 independent runs; orchestrator verified 2 more). +4 promoted cells, 0 regressions vs prior cycle baseline 104/88/0/16/0. just build / just clippy (-D warnings) / just test / just test-release / just check-textual-replace-on-codegen / just check-include-str-coverage all clean.

REVIEW GATE (cycle 121 parallel read-only):
- qa-test-runner: GO. 849 tests dev / 849 release / 0 failed / 4 e2e runs identical. Stale-symbol sweep on data_iv_indexing / collect_data_iv_indexing / walk_data_iv_indexing returned empty (clean retirement). Diff scope matched the brief exactly.
- mped-architect: GO. P0/P1 clean. P2.1 (inner-axis-leading partition + halo-strip latent shapes) FILED as TASK-0306 forward-carry. P2.2 (ambiguity-arm uncovered) APPLIED (one synthetic-fixture test added). P3.1 (UPSTREAM GUARANTEE wording too strong) APPLIED (softened + explicit open-shapes paragraph). P3.2 (test fixture duplication) acknowledged as a note, deferred (coupling two regression catches into one helper is the larger anti-pattern).

GOTCHAS + FORWARD-CARRY:
- DataflowEdge::new (synthetic test constructor) creates accesses with EMPTY indices — preserved via the None-returning fall-back path in compute_partition_bounds_with_dim_prefix. Every shipped pre-TASK-0301 transfer_inject test passes verbatim.
- The 'additive' contract semantics of TASK-0301 SHIFTED from 'iv-membership filter' to 'dim-prefix filter' but is observationally equivalent on every shipped M5 cell (every cell's partition axes ARE a prefix of every data's dim order in the shipped fixtures). The change BITES on the 07-matmul/distributed-2d cell where b's coverage = {1} is non-prefix; pre-TASK-0302 the per-symbol filter would have emitted [(j, j_band)] and wait_slice would have silently mis-sliced b's k dim.
- The OPEN shapes paragraph in multi_worker_walker.rs:937-955 explicitly tracks: (a) halo-strip bounds emission with non-outer-leading data layout; (b) inner-axis-leading partition. Both filed as TASK-0306.
- The architect's P2.2 ambiguity-arm test uses IrBinOp::Add + Ident, both ivs partitioned — picks the defensive 'drop to whole-array' arm. If a future change picks 'first partitioned iv' or 'outer iv' instead, this pin fires LOUD.
- forward-carry to TASK-0306: the 'inner-axis-leading partition' extension would need compute_partition_bounds_with_dim_prefix to emit bounds in dim order regardless of partition_axis_order, which means partition_axis_order may NOT match dim order even for non-fall-back cases. Today they happen to coincide on every shipped cell.
<!-- SECTION:NOTES:END -->
