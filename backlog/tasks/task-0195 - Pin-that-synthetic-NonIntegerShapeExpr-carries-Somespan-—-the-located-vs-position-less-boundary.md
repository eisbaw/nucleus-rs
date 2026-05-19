---
id: TASK-0195
title: >-
  Pin that synthetic NonIntegerShapeExpr carries Some(span) — the
  located-vs-position-less boundary
status: Done
assignee:
  - '@mped'
created_date: '2026-05-19 16:13'
updated_date: '2026-05-19 18:29'
labels:
  - compiler
  - diagnostics
  - tech-debt
  - test
dependencies:
  - TASK-0090
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Non-blocking coverage gap from the TASK-0090 review gate (both reviewers GO; mped-architect P3). multi_site_variants_are_position_less only pins ConstCycle as span:None. The synthetic <index/loop-bound expression> NonIntegerShapeExpr correctly carries a REAL expr.span (only its decl LABEL string is synthetic) — but NO test pins this, so a future change could silently flip it to None or a wrong span without a test biting. The TASK-0090 in-thread doc-lie fix (commit after 1c4e90a) corrected ir.rs/test prose to state only ConstCycle is position-less; this task adds the missing POSITIVE test so the located-vs-position-less boundary is enforced both ways.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 A positive test feeds a program whose loop-bound/index expression is non-integer (triggering the synthetic NonIntegerShapeExpr) and asserts the LowerError carries Some(span) at the CORRECT offset (validated via error::offset_to_line_col against the crafted source), NOT None
- [x] #2 Full gate green (just test/e2e 30/26/0/4/0/determinism byte-identical/clippy --all-targets/ci); no behaviour change
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Craft program with non-integer loop-bound: for j : 0 .. foo() { ... } where foo is a kernel call (Expr::Call in index/loop-bound position).
2. Add positive test asserting LowerError kind is NonIntegerShapeExpr with decl "<index/loop-bound expression>" AND span is Some at the call offset, validated via offset_to_line_col against crafted source.
3. Full gate; no behaviour change.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
IMPLEMENTED. Added synthetic_non_integer_shape_expr_is_located (tests/algo_lower.rs).

CRAFTED PROGRAM: a kernel call used as a loop upper bound — `for j : 0 .. f() { x[j] <-- f(); }` (f declared `kernel f : () -> usize pure;`). A kernel call in a loop-bound position is illegal; lower_index_expr hits its Expr::Call(_) arm and raises NonIntegerShapeExpr{ decl:"<index/loop-bound expression>" (the SYNTHETIC label), reason:"kernel calls are not allowed here" } located at expr.span (the call `f()`).

ASSERTS: (1) kind is NonIntegerShapeExpr with the SYNTHETIC decl label (not a real decl name) — pins the synthetic path specifically; (2) err.span is Some(..) NOT None (a None here is a real regression vs the algo/ir.rs doc that states this synthetic variant carries the real expr.span); (3) the span resolves via offset_to_line_col to the loop-bound call `f()` — expected (4,14), independently recomputed from src.find("0 .. f()")+5 so it pins the real source position, not a guessed constant. This closes the located-vs-position-less boundary positively (multi_site_variants_are_position_less pins only ConstCycle=None).

AC#2 no-behaviour-change: pure test addition; determinism byte-identical 30/26/0/4 x2.

GATE: synthetic_non_integer_shape_expr_is_located PASSES; just test 0 failed; e2e 30/26/0/4/0; clippy --all-targets clean; ci exit 0. Commit 78f266c.

FEED-FORWARD: the crafted non-integer-shape program is `for j : 0 .. f()` with f a pure kernel — simplest trigger of the SYNTHETIC-label NonIntegerShapeExpr in lower_index_expr; the body f() is never reached (bound errors first).

ORCHESTRATOR review-gate close (phase3-ralph): both reviewers GO, all 4 batch tasks, no NO-GO, no follow-up required (each independently verifiable, not a faked batch-Done). qa-test-runner: workspace 425/0; 3 new tests pass by name; determinism byte-identical x2 + e2e EXACTLY 30/26/0/4/0 + both negatives bite; clippy --all-targets clean; ci exit 0; no new panic on user paths. mped-architect (independently verified, not trusting self-report): TASK-0194 enum-removal AIRTIGHT — Expr::Ident genuinely parser-unreachable (ident_or_call routes bare ident via index_tail .repeated() to Expr::LValue empty-indices, never Expr::Ident; ZERO construction sites repo-wide), only the 4 dead delegating arms removed (the 4 helpers stay LIVE via the surviving LValue arms), NO forced _ wildcard anywhere (clean cargo check = Rust proves exhaustiveness), IrExpr::Ident is a DISTINCT live type untouched; TASK-0193 generalized message literally true for sync+async AND sync,sync AND async,async (no residual overclaim), grammar-sched.md notes 5/7/§5.3 now EXHAUSTIVE + verbatim-matching the shipped message (the recurring comment/doc-lie class NOT repeated; old "both sync and async" §5.3 quote fully removed), variant payload unchanged so no test migration/strength preserved; TASK-0195 genuinely exercises the SYNTHETIC-label NonIntegerShapeExpr path (decl=="<index/loop-bound expression>") asserting Some(span) at the source-recomputed offset (4,14) — closes the TASK-0090 located-vs-position-less boundary both ways; TASK-0197 a genuine control-flow ordering pin (multi-fault dup-worker+unknown-class asserts DuplicateWorker first + !matches UnknownWorkerClass) that WOULD fail if ref-recording moved before the dup guard — constrains the TASK-0196-equivalence invariant, not tautological. decision-0003 upheld (only non-comment src addition is the Display string); scope clean (0193 did NOT widen TASK-0086 option-span; 0194 algo-only no IR/codegen). Per-task Done honest. This cycle converted 4 review-surfaced filed follow-ups into verified Done (good graph hygiene). Task Done stands.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added a positive test pinning that the synthetic <index/loop-bound expression> NonIntegerShapeExpr carries Some(span) at the correct offset — closing the located-vs-position-less boundary (only ConstCycle is position-less).

Changes:
- tests/algo_lower.rs: synthetic_non_integer_shape_expr_is_located. Crafted program `for j : 0 .. f()` (f a pure kernel) triggers the synthetic-label variant; asserts kind+synthetic decl label, span is Some (NOT None), and offset_to_line_col resolves to the loop-bound call (4,14), recomputed from source not guessed.

No behaviour change (pure test addition); determinism byte-identical x2.

Gate: just test 0 failed; e2e 30/26/0/4/0; determinism byte-identical 30/26/0/4 x2; clippy --all-targets clean; ci exit 0. Commit 78f266c.
<!-- SECTION:FINAL_SUMMARY:END -->
