---
id: TASK-0398
title: >-
  checked_neg(i64::MIN) silent-sibling sweep: build.rs const-loop-bound
  mis-diagnosis + build.rs doc-lie + op-string pins
status: To Do
assignee: []
created_date: '2026-06-01 02:49'
updated_date: '2026-06-01 02:51'
labels:
  - hardening
  - testing
  - silent-sibling
  - doc-lie
  - diagnostics
  - cycle-235
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-235 architect review (commit eacd575) silent-sibling + doc-lie findings folded from TASK-0397. While TASK-0397 tested the algo/lower.rs checked_neg(i64::MIN) negate arms (ConstOverflow/ShapeOverflow), it did NOT grep ALL checked_neg sites (recurring defect #2 silent-sibling). The architect found two structurally-identical siblings on the IR-level evaluators:

P2.1 (the real finding) -- nucleus/nucleus-compiler/src/acfg/build.rs:558 eval_const ... .and_then(i64::checked_neg): a const-but-OVERFLOWING loop bound (e.g. 'for j : 0 .. -(0 - 9223372036854775807 - 1)') is lowered without folding (lower_index_expr lower.rs:1165 does not check), linked, then folded in build_acfg::eval_const -> checked_neg(MIN) -> None -> BuildAcfgError::NonConstLoopBound (build.rs:193/200). That reports a CONST expr as NON-CONST -- a MISLEADING diagnostic (typed, no panic, no miscompile, but wrong-typed). CHEAP-EMPIRICAL-VERIFY FIRST (the architect traced the verified code but did NOT run it end-to-end -- memory feedback-implementer-disclosure-mechanism-wrong applies to this very finding; construct the .nuc and run before fixing). FIX: add an overflow-distinct diagnostic for const-but-overflowing bounds + a prove-the-check-bites test at SITE granularity.

P2.1b -- nucleus/nucleus-compiler/src/passes/common.rs:355 eval_const_int ... .and_then(i64::checked_neg): CONFIRMED SAFE (None = graceful 'not a foldable const' fallback -> conservative non-affine path; docstring line 347 already lists overflow). Action: just document why it is safe (no behaviour change) so the next sweep does not re-flag it.

P2.2 (comment-doc-lie, recurring defect #1) -- nucleus/nucleus-compiler/src/acfg/build.rs:550 docstring says 'we panic here on None' but the call sites build.rs:193/200 return a typed NonConstLoopBound (the line-192 comment even says 'not a panic (TASK-0179)'). Fix the docstring to match (de-line-numbered, grep-locator style per the doc-fence family).

P3.2 (low, optional) -- op-string granularity: the reachable binop overflow arm carries op in {add,sub,mul,div,mod}; only 'mul' (TASK-0396) and 'negate' (TASK-0397) are asserted. add/sub/div/mod overflow strings remain unasserted. Single shared code path => low value; include only if cheap.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Cycle-235 PRE-VERIFICATION (orchestrator, cheap-empirical-verify before handoff): confirmed the FIRST half of the architect P2.1 trace empirically. Probe `for j : 0 .. -(0 - 9223372036854775807 - 1) { ... }` -> lower_algo returns Ok (overflow NOT caught at lowering; lower_index_expr does not fold/check). So the overflow DOES pass through lowering and reach the build_acfg::eval_const layer, as the architect traced. The SECOND half (does build_acfg::eval_const -> checked_neg(MIN) -> None -> BuildAcfgError::NonConstLoopBound, i.e. the mis-diagnosis) still needs an end-to-end lower->link->build_acfg run to confirm the EXACT downstream error -- do that first thing when picking this up (construct the same .nuc, drive it through build_acfg, assert the actual error variant) before designing the overflow-distinct diagnostic. Premise CONFIRMED real; exact downstream variant TBV.
<!-- SECTION:NOTES:END -->
