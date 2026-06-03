---
id: TASK-0436
title: >-
  S3/S5 overflow hardening: real-16-jacobi max-abs-diff kernel must use
  unsigned_abs/i64-widening (wrapping_sub().abs() panics/mis-ranks at i32::MIN)
status: Done
assignee: []
created_date: '2026-06-03 14:21'
updated_date: '2026-06-03 18:12'
labels:
  - grammar-extension
  - reduction
  - overflow
  - panic-not-diagnostic
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architect P2-1 (cycle-259, TASK-0341.02.01.04 S3 review). The S3 fixture kernel max_abs_acc uses acc.max(n.wrapping_sub(o).abs()) (nucleus/nucleus-compiler/tests/fixtures/task0341_s3_maxabsdiff/kernels.rs:~46). This is SOUND only for the bounded fixture input (operands [-500,-97], max abs-diff 186). It is NOT overflow-safe: when n.wrapping_sub(o) == i32::MIN (reachable e.g. n=0,o=i32::MIN), .abs() PANICS in debug and returns the NEGATIVE i32::MIN in release, which mis-ranks the max fold and BREAKS the "abs>=0 => 0 is the max-identity" invariant the whole S3 result rests on (panic-not-diagnostic recurring defect). S5/.06 (TASK-0341.02.01.06) is explicitly told the S3 fixture is a "working fixture to copy" against REAL, UNBOUNDED 16-jacobi generation-pair data. The real-data convergence reduction kernel MUST replace wrapping_sub().abs() with unsigned_abs() (-> u32, widen/compare) or i64-widening before .abs(). AC: the S5 convergence reduction kernel computes |new-old| with NO i32::MIN.abs() panic path, with a negative/extreme-input test pinning it. Do NOT copy the S3 fixture kernel verbatim.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Real-16-jacobi (S5) max-abs-diff kernel computes |new-old| with no i32::MIN.abs() panic/mis-rank path (unsigned_abs or i64-widening)
- [x] #2 A negative/extreme-input test pins the overflow-safe behaviour (e.g. n=0,o=i32::MIN yields a correct large positive abs-diff, not a panic nor a negative value)
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
DONE cycle-262 with TASK-0341.02.01.06 (commit ee5513b). The 21-jacobi-converge convergence reduction kernel (nuc-nucleus/examples/21-jacobi-converge/kernels.rs) computes |new-old| via abs_diff_i32: i64-widen both operands, unsigned_abs (->u64, NO i32::MIN.abs() panic path), clamp to i32::MAX. The reference oracle mirrors the spelling. AC#1 (no panic/mis-rank path) + AC#2 (extreme-input test): tests/task0436_overflow_safe_abs.rs include!s the EXACT shipped kernel and pins n=0,o=i32::MIN -> i32::MAX (large positive, never panic, never negative) + max_abs_acc never folds negative, in BOTH dev (just test) and release (just test-release) profiles - the release arm is load-bearing because the S3-kernel bug is silent (negative i32::MIN) in release. Did NOT copy the S3 fixture kernel.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Overflow-safe max-abs-diff: 21-jacobi-converge abs_diff_i32 uses i64-widening + unsigned_abs + i32::MAX clamp (no i32::MIN.abs() panic/mis-rank). Pinned by an extreme-input test (n=0,o=i32::MIN) that runs in dev AND release.
<!-- SECTION:FINAL_SUMMARY:END -->
