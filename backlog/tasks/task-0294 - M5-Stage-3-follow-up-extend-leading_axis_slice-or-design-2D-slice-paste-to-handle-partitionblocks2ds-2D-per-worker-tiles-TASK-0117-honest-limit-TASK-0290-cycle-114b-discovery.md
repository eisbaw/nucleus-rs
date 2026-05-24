---
id: TASK-0294
title: >-
  M5 Stage-3 follow-up: extend leading_axis_slice (or design 2D slice-paste) to
  handle partition=blocks2d's 2D per-worker tiles (TASK-0117 honest-limit +
  TASK-0290 cycle-114b discovery)
status: To Do
assignee:
  - '@mark'
created_date: '2026-05-24 23:34'
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
