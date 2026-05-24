---
id: TASK-0263
title: >-
  M5 Stage 2: transfer_inject consumes halo_widths to extend per-tile transfer
  ranges
status: To Do
assignee: []
created_date: '2026-05-24 01:40'
labels:
  - M5
  - compiler
  - halo
  - transfer
  - stage-2
dependencies:
  - TASK-0260
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Stage 2 of the TASK-0260 halo loop. Stage 1 (TASK-0260, cycle 81) landed halo inference + sidecar persistence. This task wires transfer_inject as the consumer.

## Acceptance criteria
1. transfer_inject reads halo_widths from the ACFG.
2. Each XferPlaceholder whose tile.bounds axis has a non-zero halo entry for the producer/consumer kernel has its lo/hi extended by +/- halo (clamped to source range).
3. New e2e cell 05-stencil/distributed x pthreads-async bit-identical to reference.bin.
4. Driver moves from apply_halo_inference_advisory (lenient) to apply_halo_inference (strict), OR keeps lenient with partition-policy-aware error surfacing. Choose consciously and document.

## Honest scope
- Halo on Mod / data-dependent indices remains rejected (PRD §13). Example 11 step_or_seed still has no distributed schedule.
- Block-pair recovery for partition=blocks2d is the separate Stage 3 (TASK-0264).

## Forward-carry from TASK-0260 cycle 81
- Lenient/strict split exists deliberately. Stage 1 stored a 0 entry for every (kernel, iv) the detector inspected (bare-iv case); Stage 2 must treat 0 as no extension needed.
<!-- SECTION:DESCRIPTION:END -->
