---
id: TASK-0289
title: >-
  M5 Stage 3 follow-up: halo-strip Push/Wait synthesis under partition=blocks2d
  + bit-identical e2e cell (TASK-0264 AC#3 + AC#4)
status: To Do
assignee: []
created_date: '2026-05-24 19:58'
labels:
  - M5
  - compiler
  - halo
  - partition
  - stage-3
  - forward-carried-from-TASK-0264
dependencies:
  - TASK-0264
  - TASK-0260
  - TASK-0263
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background
TASK-0264 cycle 113 landed AC#1+2 (sidecar plumbing): ACFG.partition_pairs + ACFG.grid_shape_for_outer_iv + their NameSidecar mirrors are populated by apply_partition_blocks2d. This task lands AC#3 (cross-worker halo-strip Push/Wait synthesis between neighbours in the 2D grid) and AC#4 (new bit-identical e2e cell on a 2D-partitioned stencil).

## Pre-recorded design picks (from TASK-0264 cycle 113)
- pairing recovery: sidecar.partition_pairs.get(outer_iv) returns Some(inner_iv) iff the iv-scope was partitioned by a single blocks2d directive. No re-derivation needed.
- worker -> (row, col) inversion: i = body_workers.iter().position(|w| *w == worker).unwrap(); (row, col) = (i / cols, i % cols) where (rows, cols) = sidecar.grid_shape_for_outer_iv.get(outer_iv).unwrap(). body_workers iteration is BTreeSet numeric order, matching partition_blocks2d's row-major assignment.

## Acceptance criteria
1. transfer_inject (or a new pass — pick consciously) reads sidecar.partition_pairs + sidecar.grid_shape_for_outer_iv + sidecar.halo_widths and synthesises cross-worker Push/Wait pairs for the N/S/E/W neighbour cells in the 2D worker grid. Each non-edge cell gets up to 4 halo-strip transfers (one per cardinal direction); edge cells get fewer. Corner cells (NE/NW/SE/SW) are NOT included in the first cut — they are out-of-scope per TASK-0264's task brief.
2. New e2e cell: a 2D-divisible stencil (likely a new example or a new schedule on 05-stencil with a 2x2-grid-divisible image dimension) tagged 05-stencil/distributed-2d × pthreads-async, bit-identical to a hand-written reference oracle.
3. Existing 05-stencil/distributed × pthreads-async × pthreads-sync × mp-tcp-bufsync × mp-tcp-event matrix cells must remain GREEN — the new halo-strip synthesis fires iff the iv is in sidecar.partition_pairs, which is empty for every shipped schedule pre-cycle-113. Additive-only.
4. e2e baseline at least 93/80/0/13/0 (the new cell adds +1 to total + +1 to pass).
5. just determinism-check stays green on every cell.

## Dependencies
- TASK-0264 (Done AC#1+2; this task closes AC#3+4 and lets TASK-0264 mark Done).
- TASK-0260 (halo inference Stage 1 — Done).
- TASK-0263 (transfer_inject extends per-tile transfer ranges by halo widths — Done in cycle 83; the AC#1 work would EXTEND that pass with the new halo-strip Push/Wait synthesis OR live in a new sibling pass — design pick deferred until the implementer surveys the existing extension surface).

## Honest scope
- DEEP work. Realistically a 2-3-cycle task: (a) pass extension or new pass with the neighbour-resolution + Push/Wait synthesis, (b) new schedule + new example or modified image dimensions to get 2D-divisibility, (c) implementer / review-hardening loop.
- Cycle 80 architect P2 forward-carry: TASK-0260 halo inference is partition-agnostic. The pairing + grid-shape sidecars introduced in cycle 113 are the load-bearing input to AC#3 — without them the consumer couldn't disambiguate paired-by-blocks2d ivs from independent partition=rows ivs. That decoupling is done.
- Mp-tcp-bufsync + mp-tcp-event are likely SKIP for the new 2D cell on the same w↔w-mesh basis they SKIP today's 05-stencil/distributed cell (TASK-0175); pthreads-sync + pthreads-async are the bit-identical targets for AC#2.

## Forward-carried lessons from TASK-0264 cycle 113
- Adding new ACFG / NameSidecar fields touches every pass that does destructure-and-rebuild (8 files in this codebase) PLUS every hand-built ACFG instance in tests (~14 sites). The compiler enforces this via E0063 missing-field, so the work surface is greppable but verbose. Use replace_all with a unique trailing-field pattern (reuse_widths -> reuse_widths\n + new fields).
- build_sidecar in nucleus-compiler/src/sidecar.rs is the single mirror site — add the .clone() forwarding for both new ACFG fields there.
- partition_blocks2d.rs is the ONLY populator of partition_pairs + grid_shape_for_outer_iv; the other 6 passes forward verbatim. Mirror that pattern when extending — keep the writer single-source.
- The sidecar serde round-trip + missing-field-default test (sidecar_partition_blocks2d.rs) is the wire-shape pin. Mirror that template if a future cycle adds another additive sidecar field.

## Cross-reference
- ACFG fields: nucleus/nucleus-compiler/src/acfg.rs (partition_pairs, grid_shape_for_outer_iv — added cycle 113).
- NameSidecar fields: nucleus/nucleus-compiler/src/sidecar.rs (same names).
- Writer: nucleus/nucleus-compiler/src/passes/partition_blocks2d.rs (apply_partition_blocks2d).
- Mirror: nucleus/nucleus-compiler/src/sidecar.rs::build_sidecar.
- Tests pinning the writer + wire shape: nucleus/nucleus-compiler/tests/partition_blocks2d.rs + nucleus/nucleus-compiler/tests/sidecar_partition_blocks2d.rs.
<!-- SECTION:DESCRIPTION:END -->
