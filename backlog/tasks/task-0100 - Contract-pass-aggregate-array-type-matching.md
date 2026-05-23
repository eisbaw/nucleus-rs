---
id: TASK-0100
title: 'Contract pass: aggregate / array type matching'
status: Done
assignee: []
created_date: '2026-05-18 00:52'
updated_date: '2026-05-23 21:32'
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

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED-no-driver (orchestrator-direct, cycle 77 sweep). Same pattern as TASK-0123 (closed cycle 77): the v2 codegen convention (chosen via TASK-0103 Done cycle ~64) is Vec<T> for aggregate algorithm types; every in-tree kernels.rs uses Vec<T>. Aggregate type matching (matching Nuc 'f32[H][W]' against Rust '[[f32; W]; H]' or 'Box<[[f32; W]; H]>') has zero current driver — no kernel uses those Rust shapes. The contract pass's current 'not yet implemented' message for aggregates is honest-loud: a future kernel attempting an aggregate Rust signature will get a precise message that names this task. Reopen when a real kernels.rs needs the typed-array shape (likely tier-3 / TASK-0050 territory). Same deferred-no-driver pattern as TASK-0123.
<!-- SECTION:FINAL_SUMMARY:END -->
