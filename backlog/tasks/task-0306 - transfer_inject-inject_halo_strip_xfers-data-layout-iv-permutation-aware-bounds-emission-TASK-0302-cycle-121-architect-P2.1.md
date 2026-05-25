---
id: TASK-0306
title: >-
  transfer_inject + inject_halo_strip_xfers: data-layout / iv-permutation-aware
  bounds emission (TASK-0302 cycle-121 architect P2.1)
status: In Progress
assignee:
  - '@mark'
created_date: '2026-05-25 03:55'
updated_date: '2026-05-25 09:05'
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

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Cycle 133 implementation plan (orchestrator-direct, per memory feedback-spawned-agents-refuse-code-edits):

1. Verify code pointers: confirmed inject_halo_strip_xfers at transfer_inject.rs:2523 (the 4 hardcoded [(outer_iv,...),(inner_iv,...)] tile-construction sites at lines 2721-2772 covering N/S/W/E branches); compute_partition_bounds_with_dim_prefix at line 1918. AC#2 is structurally ALREADY satisfied — the function walks per_dim in numeric dim order (not partition nest order), so it already emits in data-dim order regardless of whether outer or inner iv lands at dim 0. AC#4 becomes a verification test, not a code change.

2. Add new helper order_halo_strip_bounds_by_data_dim near compute_partition_bounds_with_dim_prefix:
   - Inputs: data, outer_iv, outer_range, inner_iv, inner_range, data_dim_iv_map.
   - When per_dim is None/empty (synthetic fixtures with DataflowEdge::new): return default [(outer_iv, outer_range), (inner_iv, inner_range)] order (preserves halo_strip_synth.rs positive_3x3 / positive_2x2 / determinism tests).
   - When both ivs at distinct dims: emit in DIM ORDER (outer_dim < inner_dim → outer first; inner_dim < outer_dim → inner first).
   - When ivs at same dim (ambiguous): return Vec::new() → whole-array drop.
   - When one or both ivs missing from data's dim union (sparse / non-prefix [k][j] case): return Vec::new() → whole-array drop.

3. Change inject_halo_strip_xfers signature to take &data_dim_iv_map: &BTreeMap<DataId, Vec<BTreeSet<IterVar>>> as new parameter. Thread it through from the call site (transfer_inject.rs:428) where the map is already computed and in scope.

4. Modify the 4 N/S/W/E emit sites (lines ~2721-2772) to call the helper and pass its result through IterTile::new. Empty bounds → wait_slice / receiver-side codegen interprets as whole-array push (defensive but correct).

5. Add tests in halo_strip_synth.rs:
   - AC#3: synthetic 2x2 grid with data indexed [inner_iv][outer_iv] (inner-leading layout) — assert halo-strip tile bounds emitted in DIM ORDER (inner first).
   - AC#4: synthetic 2x2 grid with data indexed [k][inner_iv] where k is NOT a partitioned iv (non-prefix sparse layout) — assert halo-strip tile bounds DROPPED to whole-array (empty bounds vec).
   - Tests need a new fixture builder build_2d_acfg_with_indexed_access that takes a custom data_in_access indices vec.

6. Verify M5 cells preserved byte-identical (AC#5):
   - 05/distributed-2d: img_in indexed [y][x], partition=(outer=y, inner=x). outer_dim=0, inner_dim=1 → emit (outer_iv first) — same as today. e2e baseline 108/92/0/16/0 preserved.
   - Other M5 cells don't trigger inject_halo_strip_xfers (no partition_pairs).

7. Run gate: nix develop --command bash -c 'just check && just clippy && just test && just test-release && just e2e'. e2e baseline MUST equal 108/92/0/16/0.

8. Parallel review gate: qa-test-runner + mped-architect read-only, on the commit range. Fold-back findings in-thread.

Forward-carried gotchas from TASK-0310 cycle 125 (factored into the test design):
- inject_halo_strip_xfers emits cross-worker strip Pushes; do not name new test functions with uppercase (clippy -D non_snake_case).
- The 2-Push-class disambiguation idiom (x.src ∈ partition_worker_ranges[outer_iv].keys()) is NOT needed for the new tests — these tests target build_2d_acfg fixtures with NO host-broadcast Pushes (empty data_producers in LinkedIR).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0310 cycle 125: when adding the iv-permutation-aware behaviour-pin tests AC#3 + AC#4, be aware that under partition=blocks2d transfer_inject emits TWO Push classes for each halo-bearing data: (a) host-broadcast main Pushes carrying band±halo on each axis, (b) inject_halo_strip_xfers cross-worker halo strips carrying 1-row / 1-column slices. A naive XferRole::Push && data == X_id filter catches BOTH. The cycle-125 task0310_05 idiom disambiguates by checking x.src ∈ partition_worker_ranges[outer_iv].keys() (compute worker → strip) vs not (host → main). Discovered empirically by a real test failure on cycle-125 first run; pure narrative would have missed it. See nucleus/nucleus-compiler/tests/sidecar_halo.rs:task0310_05_* for the canonical filter idiom. Also: any test name with uppercase letters (the literal _y_AND_x suffix the TASK-0310 brief proposed) fails clippy -D non_snake_case; use _y_and_x instead.

Cycle 133 implementation landed (orchestrator-direct, per memory feedback-spawned-agents-refuse-code-edits):

- AC#1: inject_halo_strip_xfers consults data_dim_iv_map via new helper order_halo_strip_bounds_by_data_dim. The 4 N/S/W/E emit sites (transfer_inject.rs ~lines 2725-2807) now call the helper; whole-array drop (empty bounds) is the safe fall-back for sparse/ambiguous/non-prefix coverage.
- AC#2: VERIFIED already-satisfied — compute_partition_bounds_with_dim_prefix walks per_dim in numeric dim order (not partition nest order), so it already emits in data-dim order regardless of inner-axis-leading or outer-axis-leading partition.
- AC#3: task0306_ac3_inner_axis_leading_layout_emits_in_dim_order — synthetic 2x2 grid with data indexed [inner_iv][outer_iv] pins that halo-strip bounds flip to (inner_iv, ...), (outer_iv, ...) — verified GREEN.
- AC#4: task0306_ac4_non_prefix_data_layout_drops_to_whole_array — synthetic 2x2 grid with data indexed [k][inner_iv] (k unpartitioned) pins that halo-strip bounds DROP to empty (whole-array push) — verified GREEN.
- AC#5: e2e baseline preserved bit-identical (108/92/0/16/0). The canonical outer-leading shipped layout (img_in[y][x] × partition_blocks2d(y,x)) takes the helper's outer_dim<inner_dim branch which is observationally a no-op. AC#5 also pinned by new task0306_ac5_canonical_outer_leading_layout_preserves_emit_order test.
- Existing positive_3x3 / positive_2x2 / determinism / placement tests (which use DataflowEdge::new with empty indices) take the helper's no-dim-info fall-back path → pre-cycle-133 emit order preserved.

Gate green this cycle (orchestrator-direct):
- just check: OK
- just clippy: OK (added #[allow(clippy::too_many_arguments)] on inject_halo_strip_xfers — already had 7 args, now 8 with data_dim_iv_map; convention matches reuse_inference.rs:624)
- just test (dev profile): 862/0/3 (was 854/0/3 cycle 125; +3 task0306_ac3/ac4/ac5 + 5 other recent tests since)
- just test-release: 862/0/3
- just check-textual-replace-on-codegen: OK
- just check-include-str-coverage: OK
- just e2e: 108/92/0/16/0 (bit-identical preservation)

Forward-carry to TASK-0306 description: AC#2 marked VERIFIED-IN-PLACE (no code change needed); the function's per-dim walk in numeric order already handles inner-leading layout correctly.
<!-- SECTION:NOTES:END -->
