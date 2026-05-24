---
id: TASK-0261
title: >-
  M5 sub-task: reuse loop option — delay-line / circular-buffer codegen for
  affine-stride loop-carried slices
status: To Do
assignee: []
created_date: '2026-05-23 23:54'
updated_date: '2026-05-24 01:40'
labels:
  - M5
  - compiler
  - codegen
  - reuse
dependencies:
  - TASK-0043
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD §6.3.3 + TASK-0043 AC#3. The reuse loop option closes 'the 2013 gap' (PRD §13): when a loop body reads grid[i-k..i] across iterations, emit a circular buffer (delay line) so each grid[i] is computed once, not k times.

## Scope
At codegen time, when a Dataflow's kernel arg indices reveal loop-carried OVERLAP (the body's reads on this iteration overlap with the previous iteration's reads), emit a delay-line: a small ring of recently-computed elements, indexed modulo the ring length.

## Acceptance Criteria
1. The reuse loop option, currently parsed but unconsumed (sched/parser.rs + sched/lower.rs:1095), now produces a real codegen artefact.
2. For each affine-stride index reuse pattern, the backend emits a delay line (circular buffer of the right length) instead of re-reading source slices.
3. The reuse semantics are restricted to affine strides only — data-dependent strides REJECTED with typed error (sibling restriction to halo inference; PRD §13).
4. A new e2e cell on example 5 or 7 with reuse in the schedule shows bit-identical output AND a measurably smaller intermediate working-set (e.g. emitted Vec capacities), verified by a new test asserting the delay line length is the access-pattern stride span.
5. Implementation notes record the honest limitation: 'reuse is rejected on data-dependent strides; the user must restructure'.

## Honest scope clarification
- Performance NOT proven in M5 — only correctness + bit-identical re-emit. PRD §11 'examples 5–7 benefit measurably' is a stretch target; this task closes the codegen path. Quantified perf improvement is M6+ scope.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carry from TASK-0260 cycle 81 (halo inference Stage 1 landed):

Both halo (TASK-0260) and reuse (this task) require the SAME affine-stride prerequisite per PRD §13. The halo-inference pass already lands an affine detector in nucleus-compiler/src/passes/halo_inference.rs — specifically the affine_decompose helper (accepts iv+b with b a const-foldable integer; rejects non-affine, strided, multi-iter-var, and DataRef-inside-index). When TASK-0261 (reuse codegen) lands, lift the affine_decompose helper to a shared pub(crate) location (likely passes/affine.rs or similar) so both halo and reuse share one detector. The HaloInferenceError enum has variants (DataDependentStride, StridedAccessNotSupported, MultipleIterVarsInIndex, NonAffineIndex) that reuse-side errors should mirror in shape.

Stage 1 driver policy worth carrying: the lenient apply_halo_inference_advisory variant exists so that pre-Stage-2, the rejection is advisory only (no e2e baseline regression). Reuse will need the same stance until its codegen is wired — record affine facts but do not fail compilation on non-affine reuse-tagged loops until the codegen consumes them.
<!-- SECTION:NOTES:END -->
