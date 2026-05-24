---
id: TASK-0282
title: 'Reuse codegen: multi-outer-coord generalisation (TASK-0269 follow-up)'
status: To Do
assignee: []
created_date: '2026-05-24 15:24'
labels:
  - M5
  - codegen
  - reuse
  - forward-carried-from-TASK-0269
dependencies:
  - TASK-0269
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0269 (cycle 103, commit e21d75e) landed real circular-buffer codegen for pthreads-sync's single-worker path. The first-cut takes the FIRST matching DataRef per (data_id, axis) as the CANONICAL outer-axes pattern. Reads with the same outer axes get rewritten to the buffer; reads with DIFFERENT outer axes stay verbatim.

For 05-stencil/reuse this means: `img_in[y-1][x-1..=x+1]` (the first 3 args to blur3) get the buffer treatment. The 6 reads at `img_in[y][x-1..=x+1]` and `img_in[y+1][x-1..=x+1]` stay verbatim — even though they ALSO have an x-axis reuse pattern that would benefit.

## Scope

Generalise the rewrite: emit ONE buffer per UNIQUE (data_id, axis, outer-coord-tuple) group. For 05-stencil/reuse: 3 buffers (one per row y-1, y, y+1). All 9 reads get the buffer treatment.

## Acceptance Criteria

1. `render_reuse_buf_decls` discovers EVERY unique outer-axes pattern per (data_id, axis) — not just the first matching one. Returns a Vec<ReuseRewriteGroup> per (data_id) where each group is one buffer.
2. Buffer ident scheme: `__reuse_buf_<data>_a<axis>_g<group_idx>` (group_idx in source-order to keep determinism).
3. Every DataRef matching ANY group gets rewritten; only DataRefs with axis index that fails `try_reuse_axis_offset` OR outer axes not matching ANY group stay verbatim.
4. 05-stencil/reuse e2e test: 9 of 9 `img_in[...]` reads in the for-x body are buffer reads (verify by a grep that counts `img_in[` lines inside the for-x block and asserts \<= 3 — only the per-iter updates + 0 verbatim reads on the optimal path).
5. Bit-identical to reference.bin preserved.

## Honest scope

- This is a PERF rewrite; correctness already met by TASK-0269.
- The 3 prologues + 3 per-iter updates triple the loop-entry overhead; the per-call read count drops from 9 to 0. Net win for any non-trivial stencil; cost analysis is M6+ scope.
- The buffer name disambiguation (group_idx) is a determinism concern — must walk in BTreeMap-ordered source position.

## Dependencies

- TASK-0269 (closed, cycle 103): the single-(data, axis) shape that this generalises.
- TASK-0270 (open): when multi-worker walker lands real codegen, this generalisation should land on BOTH paths or document the asymmetry.
<!-- SECTION:DESCRIPTION:END -->
