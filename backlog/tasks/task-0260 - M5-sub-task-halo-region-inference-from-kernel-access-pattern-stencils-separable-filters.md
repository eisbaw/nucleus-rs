---
id: TASK-0260
title: >-
  M5 sub-task: halo region inference from kernel access pattern (stencils,
  separable filters)
status: To Do
assignee: []
created_date: '2026-05-23 23:53'
updated_date: '2026-05-23 23:54'
labels:
  - M5
  - compiler
  - halo
  - inference
dependencies:
  - TASK-0043
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD §6.3.3 + §9 + TASK-0043 AC#2. The schedule does NOT state halo size; the compiler infers it from the algorithm's kernel access pattern. Required for distributed schedules on stencil-like kernels (examples 5, 6 in particular).

## Scope
At link-time or as an early pass, for each kernel invocation in a Dataflow, scan its arg indices (e.g. blur3(grid[y-1, x], grid[y, x], grid[y+1, x]) reads {-1, 0, +1} along y) and produce a per-(kernel, axis) halo width N (max |offset| across all reads). This halo annotates the IterTile / XferPlaceholder so transfer_inject emits per-tile transfers that include the halo overlap, and the partition pass knows the boundary overhead.

## Acceptance Criteria
1. A halo_inference pass (or a link-step extension) walks AlgoIR/LinkedIR Dataflows and computes per-(kernel, IterVar) halo widths.
2. The halo widths are persisted into NameSidecar (e.g. NameSidecar.halo_widths: BTreeMap<(KernelId, IterVar), u64>) or into ACFG XferPlaceholder.policy as a structured field.
3. Affine-stride indices ONLY (data-dependent strides REJECTED with a typed error per PRD §13 'reuse / halo data-dependent strides'). Spec: ''kernel arg index  is affine in ; otherwise reject.
4. transfer_inject + partition consumers use the halo to extend per-tile transfer ranges.
5. A new e2e cell on example 5 (3x3 stencil) with a distributed schedule produces bit-identical output, verifying the halo extends per-tile transfers correctly.

## Honest scope clarification
- This task's M5 deliverable is COMPILE-TIME inference + emit. Codegen for the boundary handling (clamp / wrap / panic / pad) is per-kernel Rust (the kernel author writes the boundary semantics). The compiler emits the right tile ranges; the kernel emits the right per-element semantics.
- Data-dependent stride detection: if any index is not affine in the loop variable, REJECT with a precise error and a forward-link.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
DESCRIPTION FIX (shell-expansion ate the backtick examples in AC#3): the intended spec is 'kernel arg index data[a*iv + b] is affine in iv; otherwise reject.' Where a and b are constants, iv is the loop's IterVar. A non-affine example that must be rejected: data[lookup_table[iv]] (data-dependent stride) or data[iv*iv] (quadratic, not affine). The compiler must recognise the affine pattern via the existing IndexExpr substrate; non-matches typed-reject at link or sched-lower with a precise forward-link to this task.
<!-- SECTION:NOTES:END -->
