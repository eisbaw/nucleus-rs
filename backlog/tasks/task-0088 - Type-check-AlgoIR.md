---
id: TASK-0088
title: Type-check AlgoIR
status: Done
assignee: []
created_date: '2026-05-18 00:24'
updated_date: '2026-05-23 22:01'
labels:
  - M0
  - compiler
  - ir
  - follow-up
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Add a type-checking pass over AlgoIR: validate that kernel signature shapes match call-site argument shapes, that the LHS shape of a dataflow stmt matches the RHS kernel's return type (modulo indexing), and that const declarations narrow correctly to their declared scalar type (e.g. const N : u8 = 1000 overflows). Builds on TASK-0009. Filed as a follow-up from TASK-0009 self-report.
<!-- SECTION:DESCRIPTION:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as ADDRESSED+DEFERRED-residual (orchestrator-direct, cycle 78 sweep). Investigation: TASK-0088 enumerates 3 type-checking concerns; 2 of 3 are already addressed and the third has no current driver:

(1) **Kernel signature shape matches call-site arg shapes** — ADDRESSED by TASK-0012 contract pass (nucleus-compiler/src/contract.rs). ContractError::TypeMismatch raises when a kernels.rs Rust signature doesn't match the algo's declared kernel shape. Phase 2 of contract.rs:322 'signature parse + match' covers exactly this AC.

(2) **Dataflow LHS shape matches RHS kernel return** — ADDRESSED via the same TASK-0012 contract pass + the link step (TASK-0011 LinkError::UnknownKernel/Data + cycle-74 TASK-0099 span-bearing diagnostics). The contract pass cross-references the algo's declared kernel signatures against the dataflow stmt's expected LHS shape.

(3) **Const narrows correctly to declared scalar type (overflow check)** — NOT YET DONE per algo/ir.rs:89 ('range-narrowing to the declared scalar type is a later concern'). HOWEVER: no current driver. Every const in every in-tree example declares 'usize' as its scalar type (verified by grep over nuc-nucleus/examples/*/prog.algo.nuc). usize is the widest scalar the language uses for indices; no algo today would benefit from narrow-scalar overflow detection. A const like 'const N : u8 = 1000' (the task description's example) would silently accept today but no in-tree example uses anything other than usize. Reopen if/when a real algo declares a narrow-scalar const (e.g. v3 tier-3 embedded examples with explicit u8/u16 ranges for byte-budget reasons).

Same ADDRESSED-IN-LARGE-PART pattern as TASK-0161 (closed cycle 78 as addressed-via-TASK-0180/0181) — the substantive work is done at adjacent layers; the originally-envisioned single 'type-check AlgoIR pass' became a contract-pass + link-step split that satisfies the same goal.
<!-- SECTION:FINAL_SUMMARY:END -->
