---
id: TASK-0263
title: >-
  M5 Stage 2: transfer_inject consumes halo_widths to extend per-tile transfer
  ranges
status: In Progress
assignee:
  - '@mped-architect-impl'
created_date: '2026-05-24 01:40'
updated_date: '2026-05-24 02:46'
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
FORWARD-CARRY from TASK-0260 cycle-81 review (architect P1 + P2):

When Stage 2 wiring lands (this task), harden the existing test coverage with:

1. **Advisory variant direct test** (architect F-P1): build a multi-error fixture (e.g. one Mod-indexed kernel call AND one strided-access kernel call in two different DataRefs); assert:
   - apply_halo_inference (strict): returns Err on the FIRST error encountered.
   - apply_halo_inference_advisory: returns the FULL error list AND a partial halo_widths map for the unaffected kernel calls (the lenient variant's load-bearing contract).
   Today only the strict path is asserted; the lenient/strict dichotomy is invisible to regression. Stage 2 toggles this in the driver — a regression in either direction MUST surface.

2. **Mod/Div explicit-rejection test** (architect F-P2): the cycle-81 documentation claims example-11 game-of-life's Mod-wrap is rejected by the strict detector; verified by reading prog.algo.nuc line 154-160 + the detector's affine_decompose function but NOT pinned by a dedicated test. Recommend: negative_mod_indexed_rejected fixture (a synthetic kernel call with ) asserting typed HaloInferenceError::NonAffineIndex.

3. **Stage 2 driver decision** (the AC#4 toggle): the cycle-81 driver uses apply_halo_inference_advisory (lenient) to preserve the e2e baseline. Stage 2 MUST NOT keep lenient blanket; once transfer_inject consumes halo, a missing entry for a partition=* axis becomes wrong output. Choose partition-policy-aware fatality: strict on directives that have a partition= consumer; lenient on bare directives. Document the decision + the e2e cell that bites it (the bit-identical 05-stencil/distributed cell IS the new test that promotes the decision).
<!-- SECTION:NOTES:END -->
