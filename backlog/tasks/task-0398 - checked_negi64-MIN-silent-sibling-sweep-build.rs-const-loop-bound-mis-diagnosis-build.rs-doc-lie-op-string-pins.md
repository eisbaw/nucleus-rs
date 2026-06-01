---
id: TASK-0398
title: >-
  checked_neg(i64::MIN) silent-sibling sweep: build.rs const-loop-bound
  mis-diagnosis + build.rs doc-lie + op-string pins
status: Done
assignee:
  - '@mark'
created_date: '2026-06-01 02:49'
updated_date: '2026-06-01 03:15'
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

Cycle-236 plan (orchestrator in-thread). EMPIRICAL CONFIRMATION done: drove `for j : 0 .. -(0 - 9223372036854775807 - 1)` through linked_from_inline_src -> build_acfg -> got BuildAcfgError::NonConstLoopBound with Display "loop `j` has a non-constant upper bound" -- actively MISLEADING (the bound IS constant, just overflows i64; message tells user to "use a constant bound" which they already did).

ROOT-CAUSE FIX (not a message-patch workaround): build.rs eval_const returns Option<i64>, conflating NotConst with Overflow/DivByZero. Add private ConstFoldError enum {NotConst, Overflow(op:String), DivByZero} + try_eval_const(e,consts)->Result<i64,ConstFoldError>; rewrite eval_const as try_eval_const(...).ok() wrapper (link/pipeline.rs:300 caller UNCHANGED -- only build.rs loop-bound site uses the richer result). New BuildAcfgError::OverflowingLoopBound{var,end,expr,detail} variant + Display arm; loop-bound site maps NotConst->NonConstLoopBound, Overflow/DivByZero->OverflowingLoopBound. Update the 1 exhaustive test match (tests/acfg.rs:635) + errors.rs Display (compiler exhaustiveness catches misses).

TESTS: regression test asserting OverflowingLoopBound for the overflow bound (+ div-by-zero bound) AND NonConstLoopBound STILL fires for the genuine `for j : 0 .. i` non-const case (prove the split is correct, both arms bite).

P2.2 doc-lie: build.rs eval_const docstring "we panic here on None" is FALSE (callers map None to typed errors) -> fix to describe the typed-error handling (de-line-numbered).
P2.1b common.rs:347 eval_const_int: CONFIRMED SAFE + ALREADY DOCUMENTED ("contains a DataRef / Call / overflow / div-by-zero" -> None = graceful non-affine fallback). No code change; this note IS the durable record so the next checked_neg sweep does not re-flag it.
P3.2 op-string pins: optional; include add/sub/div/mod if cheap.
GATE: build/clippy/test/test-release/e2e + parallel qa + architect.

DELIVERED + verified (cycle-236; commits 56a68da + 771e3f6; orchestrator in-thread). Root-cause fix landed:
- build.rs: ConstFoldError{NotConst,Overflow(op),DivByZero} + try_eval_const()->Result; eval_const() now a try_eval_const(..).ok() wrapper (link/pipeline.rs:300 caller behaviorally UNCHANGED -- architect traced wrapper equivalence ARM-BY-ARM, verified identical Option for every input incl i64::MIN/-1, div0, nested neg-overflow); loop_bound_error() routes NotConst->NonConstLoopBound, Overflow/DivByZero->OverflowingLoopBound.
- errors.rs: new BuildAcfgError::OverflowingLoopBound{var,end,expr,detail} + Display ("constant expression but cannot be folded ... NOT the non-const case").
- P2.2 doc-lie FIXED: eval_const "we panic here on None" was false (callers map None to typed errors) -> corrected.
- P2.1b common.rs:355: confirmed SAFE + inline TASK-0398 marker added (overflow->None is a graceful non-affine fallback; no diagnostic owed) so the next checked_neg sweep does not re-flag.

SILENT-SIBLING SWEEP COMPLETE (architect independently grepped all checked_neg/checked_div/checked_rem): exactly 3 IR-level const evaluators -- algo/lower.rs (already typed ConstOverflow/ConstDivByZero, TASK-0396/0397), passes/common.rs (safe fallback, now marked), acfg/build.rs (this fix). No 4th conflating site.

TESTS (prove the split bites BOTH directions): build_acfg_overflowing_const_loop_bound_is_overflow_error_not_nonconst + build_acfg_divide_by_zero_const_loop_bound_is_overflow_error (-> OverflowingLoopBound) + retained build_acfg_non_const_loop_bound (-> NonConstLoopBound, match now exhaustive with panic-on-other). All drive parse->lower->link->build_acfg (lower/link .expect() => proves the overflow passes through unfolded and surfaces specifically at build_acfg).

GATE (qa + architect re-ran): build clean; clippy 0/0 (doc_lazy_continuation clean on new /// blocks); test 1216/0/3 dev (+2); test-release 1215/0/3 (+2); e2e 385/328/0/57/0 unchanged. qa GO + architect GO.

REVIEW FOLD-BACK: architect P3 (clarify overflow+div-by-zero share OverflowingLoopBound by design, detail string is display-only) folded in-thread (commit 771e3f6). P3.2 op-string pins (add/sub/div/mod) correctly SKIPPED as low-value (single shared code path; negate/mul/div already asserted at this + the algo layer).

DONE: root-cause fix, GOx2, silent-sibling sweep complete, doc-lie fixed, e2e unchanged.
<!-- SECTION:NOTES:END -->
