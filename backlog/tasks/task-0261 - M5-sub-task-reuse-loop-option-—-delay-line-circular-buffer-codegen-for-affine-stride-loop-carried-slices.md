---
id: TASK-0261
title: >-
  M5 sub-task: reuse loop option — delay-line / circular-buffer codegen for
  affine-stride loop-carried slices
status: To Do
assignee: []
created_date: '2026-05-23 23:54'
updated_date: '2026-05-24 00:21'
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
Forward-carry from TASK-0258 (cycle 79c, partition_rows landed): this task is ORTHOGONAL — 'reuse' is a sliding-window optimisation, not a worker-distribution policy. Not blocked on TASK-0258, TASK-0259, TASK-0260, or TASK-0262. However:

1. **Reuse + partition_rows interaction**: a stencil schedule with both 'loop x : block=64, reuse;' (sliding-window across X) AND 'loop y : partition=rows;' (row-band across Y) is the original 05-stencil/distributed shape. The two directives compose along orthogonal axes: reuse on X (intra-worker, intra-row), partition=rows on Y (cross-worker). Verify the composition does not double-bind any sidecar field when reuse lands.

2. **Pass order**: reuse's consumer pass should sit AFTER block_transform (consumes the strip-mined inner iter_var) and is independent of partition_workers / partition_rows. Driver pipeline order in nucleus/driver/src/main.rs around line 332 is the integration point; choose a position relative to the other consumers that matches the dependency graph.

3. **TASK-0258 template applies**: typed errors at pass entry (mirror PartitionRowsError pattern), sidecar (likely new field for sliding-window state), structural pre-condition checks lived in the PASS not at sched-lower.
<!-- SECTION:NOTES:END -->
