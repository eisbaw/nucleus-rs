---
id: TASK-0294
title: >-
  M5 Stage-3 follow-up: extend leading_axis_slice (or design 2D slice-paste) to
  handle partition=blocks2d's 2D per-worker tiles (TASK-0117 honest-limit +
  TASK-0290 cycle-114b discovery)
status: Done
assignee:
  - '@mark'
created_date: '2026-05-24 23:34'
updated_date: '2026-05-25 00:28'
labels:
  - M5
  - compiler
  - backend-common
  - partition
  - blocks2d
  - stage-3
  - forward-carried-from-TASK-0290
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0290 cycle 114b ran the FIRST 05-stencil/distributed-2d (partition=blocks2d, 2x2 grid) e2e cell against pthreads-async and observed a real semantic bug: the host gather pastes each worker's 2D tile as a 1D y-range slice, dropping the x-range and overwriting adjacent workers' contributions.

## Root cause

`leading_axis_slice` in `nucleus/backend-common/src/multi_worker_walker.rs` (HONEST-PARTIAL ASSUMPTION cited in the doc comment from TASK-0117): only the tile's FIRST (leading) axis is consulted. For partition=blocks2d, each XferPlaceholder's tile has bounds `[(y, y_lo..y_hi), (x, x_lo..x_hi)]` — both axes are needed to compute a 2D slice-paste. Today the x-range is silently dropped.

## Concrete defect

For a 16x16 image (256 i32 elements) under 2x2-grid partition with halo=1:
- w0 owns y=1..8, x=1..8. The compute body writes `img_out[y*16+x]` for y in 1..8, x in 1..8 — a 7x7 sub-rectangle within w0's local buffer.
- The Push/Wait pair's tile is `[(y, 1..8), (x, 1..8)]`.
- `leading_axis_slice` reads only the FIRST bound: `(y, 1..8)` ⇒ `lo=1, hi=8, stride=16` ⇒ `copy_from_slice(&_tmp[16..128])`.
- The host pastes elements 16..128 (rows 1..8, all 16 columns) — including columns 8..15 which w0 never wrote (default zero) AND including columns 0..1 which are boundary (also zero, OK).
- Then w1 (y=1..8, x=8..15) overwrites the SAME range `img_out[16..128]` with w1's local buffer, in which columns 0..8 are zero. Net result: rows 1..8 of img_out are partially zero in the wrong columns.

## Diagnostic evidence

`nucleus/target/e2e-matrix/run-*-*/05-stencil__distributed-2d__pthreads-async/src/main.rs` lines ~190-193 (host gather): four `ring.wait()` calls all targeting `img_out[16..128]` (rows 1..8) and `img_out[128..240]` (rows 8..15). Each call overwrites the prior. Diff vs reference: first divergence at byte 68 (i.e. element 17 = row 1, col 1 = first compute pixel).

## Acceptance criteria

1. `leading_axis_slice` (or a sibling 2D path) emits a slice-paste that respects ALL axes of an N-D tile. The host gather for 2D partition tiles writes only the rectangular sub-region the corresponding worker wrote.
2. Existing 1D partition=workers / partition=rows behaviour stays bit-identical (additive-only — the new 2D path fires iff the tile has 2 or more bounds AND the data dims has 2 or more axes).
3. `05-stencil/distributed-2d × pthreads-async` cell promotes from `[[skip]]` to `[[required]]` (bit-identical against 05-stencil/reference.bin).
4. e2e baseline bumps to `93/80/0/13/0` (the cell currently SKIPs on this prerequisite per TASK-0290 cycle 114b matrix entry).

## Honest scope

- Backend-common change touches both pthreads-async and mp-tcp-event renderers (multi_worker_walker is the shared codegen home — TASK-0244 cycle 37). Either both backends inherit the fix automatically OR the per-backend assign path needs updating.
- The 2D slice-paste shape itself: simplest is a nested-loop slice-paste (`for y in y_lo..y_hi { name[y*W+x_lo..y*W+x_hi].copy_from_slice(&_tmp[y*W+x_lo..y*W+x_hi]); }`). More efficient would be a per-row memcpy via `copy_from_slice` with computed offsets. Pick consciously based on the rendered output's readability vs the bit-identical wins it unlocks.
- The push side also needs review: the worker's `ring.push(img_out.clone())` sends its WHOLE local buffer (not just the rectangle it wrote). For correctness this is fine (zeros outside the rectangle), but the 2D slice-paste on the receiver must extract only the rectangular sub-region.
- Halo-strip Push/Wait pairs (TASK-0289 cycle 114a) have the same 2D-tile shape but are worker-to-worker, not worker-to-host. The receiving worker's Wait should also extract a 2D sub-rectangle from the sender's whole local buffer — same fix applies.

## Forward-carried from TASK-0290 cycle 114b

The 05-stencil/distributed-2d schedule file is landed at `nuc-nucleus/examples/05-stencil/schedules/distributed-2d.sched.nuc`. The matrix entry SKIPs pthreads-async with the citation "TASK-0294: leading_axis_slice ignores the inner axis of 2D partition tiles; host gather pastes each worker's 1D y-band slice over the prior worker's contribution".

## Cross-reference

- nucleus/backend-common/src/multi_worker_walker.rs::leading_axis_slice (the HONEST-PARTIAL).
- nucleus/backend-common/src/multi_worker_walker.rs::render_wait_assign (the consumer).
- nucleus/nucleus-compiler/src/passes/transfer_inject.rs (the tile-shape producer — already emits 2D tiles via TASK-0263 + TASK-0264).
- nuc-nucleus/examples/05-stencil/schedules/distributed-2d.sched.nuc (the fixture).
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 115 — landed

`backend-common::multi_worker_walker` extended: `LeadingAxis` pub struct → module-private `WaitSlice` enum with `Flat` (1D pre-TASK-0294 path, bit-identical-pinned) + `Rows` (new 2D row-loop slice-paste). `leading_axis_slice` → `wait_slice`; `render_wait_assign` dispatches.

Per-AC outcome:
- AC#1 (wait_slice respects all axes of an N-D tile up to rank 2): MET. `bounds.len() >= 2 && ty.dims.len() >= 2` enters `Rows` with outer/inner ranges + row_stride + inner offsets. Rank-3+ is rejected loud with a typed `EmitError::ContractGap` (cycle-115 architect P2.1 — file follow-up TASK-0295).
- AC#2 (1D partition=workers/rows stays bit-identical): MET. Existing flat-emit string preserved verbatim; `flat_1d_slice_paste_for_partition_workers` test pins the exact pre-TASK-0294 emit; `just e2e` shows the row-band distributed cells (e.g. 05-stencil/distributed × pthreads-async) still pass byte-identical.
- AC#3 (05-stencil/distributed-2d × pthreads-async promoted to [[required]]): MET. nuc-nucleus/e2e-matrix.toml updated; cell passes bit-identical against 05-stencil/reference.bin.
- AC#4 (baseline bump): MET. 96/80/0/16/0 (prior 96/79/0/17/0; the +4 distributed-2d cells were added in cycle 114b as [[skip]]; cycle 115 flips 1 skip → required pass).

## Verification

`just ci` exit 0. Full hard gate green:
- `just build` clean (workspace).
- `just clippy` clean.
- `just test` 842/0/3.
- `just test-release` 842/0/3 (release matches dev — no debug_assert! divergence).
- `just e2e` 96/80/0/16/0.
- `just determinism-check` 96/80/0/16 (matches).
- Negative arms (xbackend, required-coverage) both correctly bit on injected corruption.

New tests (nucleus/backend-common/tests/wait_assign_slice.rs, 7 tests):
- whole_array_assign_when_tile_empty
- flat_1d_slice_paste_for_partition_workers (bit-identical regression pin for the 1D arm)
- rows_2d_slice_paste_for_partition_blocks2d (the load-bearing 2D arm)
- degenerate_2d_full_range_collapses_to_whole_array (defensive emit-identity guard)
- inner_axis_out_of_bounds_returns_contract_gap (cycle-115 typed-error surface)
- leading_axis_out_of_bounds_returns_contract_gap (architect P3.2 — closes pre-existing test gap)
- rank_3_or_higher_tile_returns_contract_gap (architect P2.1 — pins the cycle-115 fail-loud guard against silent rank-3+ partial)

## Cycle-115 review-gate hardening applied in-thread

- P2.1 (architect): added rank-3+ ContractGap surface in wait_slice. Guards against the SAME HONEST-PARTIAL class the cycle-115 fix removed for 2-axis data. Pinned by rank_3_or_higher_tile_returns_contract_gap.
- P3.2 (architect): added leading_axis_out_of_bounds test (5 lines). Closes pre-existing test gap.
- P3.3 (architect): rewrote the _y/_r safety doc-comment to cite block-shadowing (placement-independent) rather than the placement-fragile original.
- P3.4 (architect): labelled degenerate_2d_full_range_collapses_to_whole_array as DEFENSIVE-ONLY in its docstring.
- P3.1 (architect): filed as TASK-0295 (sibling-promotion audit for non-pthreads-async backends).

## Forward-carried lessons (for TASK-0290 / TASK-0289 / TASK-0264 closure + any future N-D slice-paste work)

1. The cycle-115 axis-mapping assumption `tile.bounds[i].iter_var ↔ ty.dims[i]` is row-major / nest-order; held verified via rewrite_partition_tiles_inner (transfer_inject.rs:1627-1687) walking partition_axis_order outer-to-inner AND partition_blocks2d.rs:443 inserting partition_pairs as (outer_iv, inner_iv). For a hypothetical inner-axis-leading partition or non-row-major data layout the slice would silently address the wrong axis. Same HONEST-PARTIAL lineage as TASK-0117.

2. `saturating_mul` on usize offsets is defensive belt-and-braces — preceding bounds checks already rule out overflow. Harmless.

3. The single-pass box blur is invariant under partition SHAPE (1D row-band vs 2D grid): each output pixel is a pure function of its 3x3 input neighbourhood, no cross-dependencies. cycle 114b's reference-oracle-is-degenerate analysis (forward-carried from TASK-0290) is the right way to handle the bit-identical assertion when the algorithm is partition-invariant — DON'T rebuild a custom oracle; reuse the existing reference.bin from the 1D-partition sibling.

4. The shared `multi_worker_walker` means the bug fix lands in ONE place and benefits all 4 tier-1 backends automatically. The flip side: the silent-sibling pattern can spread quietly — file explicit promotion tasks (TASK-0295) when downstream blockers unlock.

5. `backend-common` edits + nucleus driver: cargo's mtime invalidation has missed this combo once before (TASK-0270 cycle 104 — see MEMORY.md). When running `just e2e` after a backend-common edit, double-check the driver release binary is newer than the source; if not, force `cargo build --release --workspace`. (Cycle 115 was fine — `just ci`'s sequencing forces the release build before e2e.)
<!-- SECTION:NOTES:END -->
