---
id: TASK-0347
title: >-
  ACFG + link: handle identity-copy dataflow statements (reopen of
  TASK-0111/0097 DEFERRED trigger fired by 15-transpose)
status: To Do
assignee: []
created_date: '2026-05-27 12:45'
updated_date: '2026-05-28 01:01'
labels:
  - compiler
  - ir
  - deferred-trigger-fired
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
=== Filed as cycle-204 forward-carry from 15-transpose discovery ===

TASK-0111 (ACFG: handle identity-copy dataflow statements) was closed cycle 77 as DEFERRED-until-real-example with the rationale "no in-tree example uses identity-copy dataflow syntax" and the closure note "reopening means filing a single fresh scope-derived task that covers both layers at once (not reopening these two separately)".

Cycle 204 added 15-transpose, which is the FIRST in-tree example with identity-copy SEMANTICS (axis-swap permutation with no per-element compute). The cycle-77 deferral trigger is now fired. Per the closure note, this is the canonical "single fresh scope-derived task" reopening both ACFG and link sides at once.

## Current workaround in 15-transpose

The example uses a `kernel xpose : (i32) -> i32 pure` identity passthrough so the dataflow assignment `output[j][i] <-- xpose(input[i][j])` is a Call RHS (which `acfg::build::build_dataflow` accepts as an Operation node). The bare-LValue form `output[j][i] <-- input[i][j]` would be the natural syntax but produces no ACFG operation today.

## Scope

Both layers must be co-designed (per cycle-77 closure note):

1. **ACFG layer** (`nucleus/nucleus-compiler/src/acfg/build.rs:325-326` "Identity copy or pure-expression RHS: skipped at M1"): identity-copy dataflow should produce an Operation node with no kernel firing but with a `data move` DataflowEdge. The Operation's worker set is the LHS's worker placement; the DataflowEdge's `data_in` is the RHS's data ref.
2. **Link layer** (TASK-0097 was the parallel limitation, also closed DEFERRED cycle 77): when the producer and consumer worker sets differ, the data move lowers to a Xfer/Push/Wait pair, same as for a kernel-call Operation.

## Acceptance Criteria
<!-- AC:BEGIN -->
1. **ACFG**: `build_dataflow` accepts bare-`LValue` RHS and emits an Operation with no kernel firing + a `data move` DataflowEdge. Unit test fixture exercising `out <-- in` with both same-worker and cross-worker placements.
2. **Link / codegen**: cross-worker data-move lowers to the same Xfer pair a Call would. Same-worker case lowers to an in-place assignment (or is structurally elided if the LHS and RHS are the same DataId).
3. **15-transpose simplification**: when AC#1 + AC#2 close, 15-transpose's prog.algo.nuc can drop the `xpose` kernel and use the bare-LValue form; the new naive schedule emits identical output.bin (regression-pin the bit-identity).
4. **Renumber/coordinate followup**: if implementer chooses, can ALSO close TASK-0097's original concern (link-side identity-copy gap).

## Honest scope LIMITS

- The bare-`LValue` syntax is grammar-legal today; the lowering pass `IrStmt::Dataflow` already carries it. Only ACFG-build + downstream codegen are gated.
- AC#3's regression pin only fires after both AC#1 + AC#2 close. If only AC#1 lands, document the half-state in the cycle close note and file AC#2 as an explicit follow-up.

## Forward-carry

- Predates cycle 204: TASK-0111 (Done DEFERRED cycle 77, ACFG side), TASK-0097 (Done DEFERRED cycle 77, link side).
- Triggered by: cycle 204 TASK-0341.01 (15-transpose AC#1) — first real example with identity-copy semantics.
<!-- SECTION:DESCRIPTION:END -->

- [ ] #1 ACFG: build_dataflow accepts bare-LValue RHS and emits an Operation with no kernel firing + a 'data move' DataflowEdge. Unit test fixture exercising 'out <-- in' with both same-worker and cross-worker placements
- [ ] #2 Link / codegen: cross-worker data-move lowers to the same Xfer pair a Call would. Same-worker case lowers to an in-place assignment (or is structurally elided if the LHS and RHS are the same DataId)
- [ ] #3 15-transpose simplification: when AC#1 + AC#2 close, 15-transpose's prog.algo.nuc can drop the xpose kernel and use the bare-LValue form; the new naive schedule emits identical output.bin (regression-pin the bit-identity)
- [ ] #4 Coordinate followup: if implementer chooses, ALSO close TASK-0097's original concern (link-side identity-copy gap, parallel limitation Done DEFERRED cycle 77)
<!-- AC:END -->
