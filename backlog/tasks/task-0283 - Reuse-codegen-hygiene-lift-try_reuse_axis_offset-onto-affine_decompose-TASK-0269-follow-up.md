---
id: TASK-0283
title: >-
  Reuse codegen hygiene: lift try_reuse_axis_offset onto affine_decompose
  (TASK-0269 follow-up)
status: To Do
assignee: []
created_date: '2026-05-24 15:49'
labels:
  - reuse
  - hygiene
  - follow-up
  - TASK-0269
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architect P2.5 from TASK-0269 cycle-103 review: nucleus/backend-common/src/render.rs::try_reuse_axis_offset(e, iv_name) at render.rs:1015+ re-implements a subset of nucleus_compiler::passes::reuse_inference::affine_decompose. Both decode 'iv + b' for the same iv, but the render-side helper takes the iv by name (a String comparison on IrExpr::Ident) while the inference-side takes the IterVar directly. The two MUST stay consistent — any future widening of the affine grammar (e.g. constant Mod folding for example 11) needs to be applied in both sites or the codegen rewrite will skip reads that inference accepted.

## Scope (one of two paths)
1. Make passes::reuse_inference::affine_decompose pub (or move it to passes::common alongside the lifted version from TASK-0261 cycle 82); have render.rs call it directly. Need a thin name-to-IterVar shim since render-time only has names.
2. Add a unit test pair in passes::reuse_inference asserting affine_decompose(e, iv) and try_reuse_axis_offset(e, name_of(iv)) produce the same Some/None decision on a representative set of expressions. Cheaper but only catches divergence at test time.

## Acceptance
- Either (1) achieves single source of truth, or (2) adds explicit cross-pass divergence detection.
- Reuse-axis offset decoding has ONE behaviour-defining site OR a regression test that bites if two diverge.

## Honest scope
This is hygiene, NOT a current correctness defect — the two helpers agree today on every shipped fixture. The risk surfaces when (a) the affine grammar widens (TASK-0260's constant-Mod-fold path for example 11), or (b) a new reuse-axis index shape is introduced. File for TASK-0270 cycle or the multi-outer-coord task TASK-0282 (whichever lands first) to address before either materially extends the grammar.

## Dependencies
None (independent hygiene).
<!-- SECTION:DESCRIPTION:END -->
