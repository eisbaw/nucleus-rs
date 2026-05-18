---
id: TASK-0012
title: Kernel-as-Rust-function contract verification
status: To Do
assignee: []
created_date: '2026-05-17 23:03'
labels:
  - M0
  - compiler
  - language
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD §6.2.2: a kernel declared in *.algo.nuc as 'kernel blur3 : (f32, f32, ...) -> f32 pure' must have a matching Rust function in kernels.rs. Implement the contract check: compile kernels.rs as part of nucleus build, verify each declared signature matches a function with the same name and signature shape.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 When building an example, nucleus invokes 'cargo check' on examples/NN/kernels.rs and parses any signature mismatch into a structured error.
- [ ] #2 Mismatch types: missing function, arity mismatch, type mismatch, missing pub modifier.
- [ ] #3 Purity is not enforced at the Rust level (rustc can't prove it). 'where pure' is a contract the user upholds; misuse is a v2 limitation noted in PRD.
- [ ] #4 Test: a deliberately mismatched kernels.rs produces a structured error pointing at the algo declaration and the Rust signature.
- [ ] #5 Implementation notes record design questions (e.g. should pure kernels be wrapped in a marker trait at codegen time).
- [ ] #6 Implementation notes record honest limitations (the purity attribute is documentation; v2 does not statically verify it).
<!-- AC:END -->
