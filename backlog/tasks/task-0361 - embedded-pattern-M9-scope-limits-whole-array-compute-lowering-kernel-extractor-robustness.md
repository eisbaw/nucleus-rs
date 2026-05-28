---
id: TASK-0361
title: >-
  embedded-pattern M9 scope limits: whole-array-compute lowering +
  kernel-extractor robustness
status: To Do
assignee: []
created_date: '2026-05-28 11:30'
labels:
  - M9
  - backend
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Forward-link follow-ups carved out of TASK-0047 (M9 embedded-pattern landing). The M9 backend lands compile-only single-worker for examples 1+5. These are the documented scope LIMITS that returned a precise EmitError::UnsupportedFeature rather than mis-lowering:

1. WHOLE-ARRAY PURE COMPUTE FIRING (non-indexed output WITH inputs). The naive single-worker schedules of ex1/ex5 do not produce this shape (their pure kernels write indexed outputs `c[i] <-- k(..)`). A kernel returning a whole array (`out <-- transform(in)`) needs an aggregate-return binding contract under the fixed `[T; N]` no_std layout. Site: backends/embedded-pattern/src/lib.rs render_fire `Some(o) if o.indices.is_empty()` arm with non-empty inputs.

2. KERNEL-EXTRACTOR TEXTUAL ROBUSTNESS. backends/embedded-pattern/src/kernel_extract.rs uses a simple brace-matcher (no tokeniser). A `{`/`}` inside a string/char/comment literal INSIDE a pure kernel body would miscount. The tier-1 pure kernels (add, blur3) contain none; the extractor returns None (loud ContractGap) on imbalance rather than emitting truncated source. A kernel body needing string/char braces would require the full-parser path.

3. ALLOC/FREE + PUSH/WAIT/SYNC events are rejected (single-worker naive carries none). Region-placed data (`place_data D in tcm_per_core`) is M10 shim work (TASK-0048); cross-MCU transfers + barriers are M11 (TASK-0049).

Pick up if/when a tier-3 example exercises any of these shapes. None blocks M9 (all are precise loud rejections, not silent mis-lowerings).
<!-- SECTION:DESCRIPTION:END -->
