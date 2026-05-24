---
id: TASK-0260
title: >-
  M5 sub-task: halo region inference from kernel access pattern (stencils,
  separable filters)
status: To Do
assignee: []
created_date: '2026-05-23 23:53'
updated_date: '2026-05-24 00:20'
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

Forward-carried from TASK-0258 (cycle 79c, partition_rows consumer landed): halo inference now has a real upstream signal to consume.

1. **Consumes ACFG::partition_worker_ranges**: partition_rows (and partition_workers) write per-(IterVar, WorkerId) row-band ranges into this BTreeMap. Halo inference needs to read it to know which workers own which row-bands and which neighbours need halo strip synthesis. The sidecar key set (BTreeMap<IterVar, BTreeMap<WorkerId, Range<i64>>>) is the natural interface — no new sidecar field needed for the partition→halo handoff.

2. **Stencil semantics**: a 3x3 box blur on a row-banded outer y-loop means each worker reads y in [band_lo-1 .. band_hi+1] (the halo width is 1 on each side, the kernel access pattern reveals this). Halo inference must synthesise Push/Wait pairs between adjacent workers for those halo strips. The transfer_inject pass (TASK-0117) is the natural integration point; partition_worker_ranges + the kernel access pattern (Operation.dataflow) is the input.

3. **05-stencil/distributed**: TASK-0258 left the schedule's 'loop y : partition=rows;' directive in commented form because the algo's y=1..15 length-14 doesn't divide by 4 workers. TASK-0262 (remainder policy) is the unblocker for restoration; once landed AND your halo inference lands, the cell can finally become [[required]] and bit-identical to reference.bin.

4. **e2e cell is YOUR task's deliverable**: TASK-0258 AC#5 (new e2e cell exercising partition=rows + bit-identical reference.bin) is explicitly BLOCKED-ON-TASK-0260. When you land halo inference, this becomes a joint deliverable: 05-stencil/distributed × at-least-one-tier-1-backend bit-identical to reference.bin. Acceptance for that cell crosses TASK-0258 / TASK-0260 / TASK-0262. Pure partition=rows without halo produces wrong output at row-band boundaries; no e2e cell can pass without halo synthesis.
<!-- SECTION:NOTES:END -->
