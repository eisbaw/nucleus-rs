---
id: TASK-0088
title: Type-check AlgoIR
status: To Do
assignee: []
created_date: '2026-05-18 00:24'
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
