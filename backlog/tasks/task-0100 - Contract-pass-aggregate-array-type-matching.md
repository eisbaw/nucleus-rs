---
id: TASK-0100
title: 'Contract pass: aggregate / array type matching'
status: To Do
assignee: []
created_date: '2026-05-18 00:52'
labels:
  - M1
  - compiler
  - language
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Extend nucleus/compiler/src/contract.rs to match aggregate Nuc types (f32[H][W], etc.) against Rust types (&[[f32; W]; H], Box<[[f32; W]; H]>, flat &[f32], etc.). v2-initial only matches scalars; aggregates are reported as TypeMismatch with a 'not yet implemented' message. This is a known stub from TASK-0012. Needs a stable convention for how the codegen pass marshals arrays so the match can be principled rather than ad-hoc.
<!-- SECTION:DESCRIPTION:END -->
