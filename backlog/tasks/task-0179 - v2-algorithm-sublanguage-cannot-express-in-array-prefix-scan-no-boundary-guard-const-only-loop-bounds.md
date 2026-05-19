---
id: TASK-0179
title: >-
  v2 algorithm sublanguage cannot express in-array prefix scan (no boundary
  guard, const-only loop bounds)
status: To Do
assignee: []
created_date: '2026-05-19 01:13'
labels:
  - M3
  - language
  - findings
dependencies:
  - TASK-0039
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Surfaced by TASK-0039 (example 04 prefix-sum). Three concrete v2 limitations make a textbook in-array carried prefix scan inexpressible: (1) carried shifted index out[i-1] underflows usize at i=0 and there is no conditional (PRD 6.2.4) to guard it; single-assignment (keyed by symbol name) forbids a base-case + loop split on the same array. (2) Loop bounds must be compile-time const (acfg.rs:697 eval_const) and PANICS rather than returning a clean LowerError on a non-const bound — triangular loops impossible AND the failure mode is an ugly panic not a diagnostic. (3) Single-assignment ignores differing constant indices so block unrolling as separate statements is rejected. TASK-0039 worked around this in-language by pushing the carry/boundary logic into hand-written Rust kernels (legal) over a rectangular reduction-accumulator; this task tracks the underlying language gaps. Options: add a clamp/saturating index intrinsic, an exclusive-scan/segmented-scan algorithm builtin, or a guarded-first-iteration form; at minimum convert the acfg.rs:697 panic into a LowerError.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 acfg.rs non-const loop bound returns a LowerError (not panic!)
- [ ] #2 Decision recorded (decision doc or PRD note) on whether in-array prefix scan gets language support or stays a kernel-level idiom
- [ ] #3 If supported: an example expresses prefix scan WITHOUT pushing the boundary into a kernel
<!-- AC:END -->
