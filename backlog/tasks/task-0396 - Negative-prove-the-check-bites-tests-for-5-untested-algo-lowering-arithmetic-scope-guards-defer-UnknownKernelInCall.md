---
id: TASK-0396
title: >-
  Negative prove-the-check-bites tests for 5 untested algo-lowering
  arithmetic/scope guards (+ defer UnknownKernelInCall)
status: Done
assignee:
  - '@mark'
created_date: '2026-06-01 02:01'
updated_date: '2026-06-01 02:18'
labels:
  - hardening
  - testing
  - prove-the-check-bites
  - cycle-234
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-234 hardening (endgame 'prove the check bites' dimension). A systematic prove-the-check-bites audit of all 113 typed-error variants across the compiler's ~24 *Error enums found 6 fail-loud guard variants CONSTRUCTED on live error paths with ZERO negative test proving they fire.

ALGO-LOWERING (reachable from .nuc source via tests/algo_lower.rs lower_str harness -- THIS task delivers these 5):
- LowerErrorKind::ConstOverflow (algo/lower.rs:565,578) -- i64 overflow in a const expr (checked_neg / checked_binop).
- LowerErrorKind::NonIntegerConstExpr (algo/lower.rs:593,607) -- kernel-call or data-ref inside a const expr.
- LowerErrorKind::ShapeOverflow (algo/lower.rs:661,674) -- i64 overflow in a shape dimension expr.
- LowerErrorKind::ShapeDivByZero (algo/lower.rs:678) -- division-by-zero in a shape dimension expr.
- LowerErrorKind::IterVarShadowsDecl (algo/lower.rs:939) -- for-loop iter var name collides with a declared const/data/kernel.

These sit beside ALREADY-COVERED siblings (ConstDivByZero, NonIntegerShapeExpr, NonPositiveDim, ConstCycle) in the SAME const/shape evaluator -- a coherent coverage hole in the arithmetic/scope guard cluster. Risk of leaving untested: a refactor of the construction condition (or an earlier check shadowing the path) could silently make a guard dead-in-practice with nothing to catch it.

DEFERRED to a follow-up (different category): HaloInferenceError::UnknownKernelInCall (halo_inference.rs:1211) -- the code itself documents it DEFENSIVELY UNREACHABLE ('would never reach this variant in practice; every production callsite checks name_kernels before halo_inference runs'). A contrived input test would be wrong; needs a white-box reachability investigation (genuinely-defensive vs dead vs opacity-gate-rot subsumed by name_kernels validation). Filed separately.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Cycle-234 plan (orchestrator in-thread per feedback-spawned-agents-refuse-code-edits; independence preserved at read-only review gate). Add 5 negative tests to nucleus/nucleus-compiler/tests/algo_lower.rs (the LowerErrorKind negative-test home, "Bonus negative coverage" section), mirroring const_divide_by_zero_is_error / non_positive_shape_dim_is_error template (lower_str(src).map_err(|e| e.first().clone()) -> match LowerErrorKind). Crafted minimal .nuc sources, each verified to make the target the PRIMARY (first) error:
- ConstOverflow: const Q : usize = 9223372036854775807 * 2;  (checked_binop mul overflow; assert in_const=="Q", op=="mul")
- NonIntegerConstExpr: const Q : usize = nope();  (Expr::Call arm; assert in_const=="Q", reason mentions kernel calls)
- ShapeOverflow: data x : f32[9223372036854775807 * 2];  (assert decl=="x", op=="mul")
- ShapeDivByZero: data x : f32[4 / 0];  (assert decl=="x")
- IterVarShadowsDecl: const N + for N : 0 .. 4 { x <-- f(); }  (loop var shadows const; shadow check returns None before body descent; assert var=="N", shadows=="N")
GATE: nix develop -c just build clippy test test-release e2e + parallel read-only qa-test-runner + mped-architect.

DELIVERED + verified (cycle-234, commit e7bf3df; orchestrator in-thread, independence at read-only gate). 5 negative tests added to tests/algo_lower.rs, all biting:
- const_expr_i64_overflow_is_error -> ConstOverflow{in_const:"Q",op:"mul"}
- non_integer_const_expr_kernel_call_is_error -> NonIntegerConstExpr{in_const:"Q",reason~kernel call}
- shape_dim_i64_overflow_is_error -> ShapeOverflow{decl:"x",op:"mul"}
- shape_dim_divide_by_zero_is_error -> ShapeDivByZero{decl:"x"}
- iter_var_shadowing_a_decl_is_error -> IterVarShadowsDecl{var:"N",shadows:"N"}

BITE-PROOF: orchestrator mutation-tested IterVarShadowsDecl (disabled the shadow guard -> test FAILED with Ok(AlgoIR), proving it catches the regression). Architect independently dumped each crafted source: every one produces a SINGLE-element error vector (the asserted variant + asserted fields, zero cascade), so .first() is unambiguous and the other 4 bite by identical construction.

GATE (qa + architect re-ran, NOT implementer-claimed): build clean; clippy 0/0 (doc_lazy_continuation structurally N/A -- new comments are // not ///); test 1211/0/3 dev (+5); test-release 1210/0/3 (+5); e2e 385/328/0/57/0 unchanged. qa GO + architect GO. Architect completeness-confirmed: all 19 LowerErrorKind variants now have >=1 negative test (was 5 missing); pre-diff audit verified against HEAD.

HONESTY NOTE (architect P3, variant-vs-site granularity -- disclosed, not hidden): ConstOverflow + ShapeOverflow each have TWO construction sites. The tests cover the reachable binop arm (lower.rs:578/674). The checked_neg (negate) arm (lower.rs:565/661) is DEFENSIVELY UNREACHABLE from .nuc source (needs i64::MIN, which exceeds the parser parse::<i64>() limit) -- same class as UnknownKernelInCall. prove-the-check-bites at VARIANT granularity (the deliverable) is fully met; the negate SITE-granularity gap is FOLDED INTO TASK-0397 (white-box reachability), not silently implied covered.

DONE: all 5 target variants have a biting negative test, gate green, GOx2, deferred halo guard filed (TASK-0397, scope-expanded to the negate sites).
<!-- SECTION:NOTES:END -->
