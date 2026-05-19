---
id: TASK-0194
title: >-
  algo::ast::Expr::Ident is parser-unreachable dead-at-construction — remove or
  document as reserved
status: Done
assignee:
  - '@mped'
created_date: '2026-05-19 15:48'
updated_date: '2026-05-19 18:29'
labels:
  - compiler
  - language
  - tech-debt
  - cleanup
dependencies:
  - TASK-0082
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Surfaced (non-blocking, optional) by the TASK-0082 mped-architect review gate. nucleus/compiler/src/algo/ast.rs Expr::Ident is never constructed by the parser: parser.rs ident_or_call always routes a bare identifier through index_tail (.repeated(), possibly empty) producing Expr::LValue(IndexedLValue{indices:[]}), never Expr::Ident. It is handled defensively in lower.rs. This PREDATES TASK-0082 (that task only re-typed the variant payload to the Spanned ident type; it did not introduce the dead-ness) — so it is latent dead/confusing surface, not a regression. Resolve: either remove Expr::Ident (and the defensive lower.rs arm) if genuinely unreachable, OR add a doc comment marking it an intentional reserved variant with the reason. Keep behaviour identical (it is unreachable, so removal/doc is no-behaviour-change); full gate must stay green.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Expr::Ident is either removed (with its now-dead lower.rs handling) OR documented in ast.rs as an intentional reserved variant with rationale; the parser-unreachability is verified (grep/test)
- [x] #2 Zero behaviour change: just test green, e2e 30/26/0/4/0, determinism byte-identical, clippy --all-targets clean, ci exit 0
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Verify Expr::Ident parser-unreachability: grep confirms NO Expr::Ident construction anywhere; parser ident_or_call always routes bare ident through index_tail (.repeated, empty) -> Expr::LValue(IndexedLValue{indices:[]}). Proven unreachable.
2. Remove Expr::Ident variant from ast.rs AND its 4 dead lower.rs arms (lines ~269/365/675/718). The LValue empty-indices arms already handle the real bare-ident path. Chosen over documenting-as-reserved: provably unreachable -> removal is cleaner, less dead surface.
3. Full gate; determinism byte-identical proves zero behaviour change.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
IMPLEMENTED. Decision: REMOVED Expr::Ident (chosen over document-as-reserved — provably parser-unreachable, so removal is the cleaner, less-dead-surface call).

UNREACHABILITY PROOF: (1) repo-wide grep finds ZERO `Expr::Ident(` construction sites (only the dead lower.rs match arms read it; `IrExpr::Ident` in algo::ir is a DISTINCT live type, untouched). (2) parser.rs::ident_or_call is the ONLY bare-ident producer: it does `.then(call_tail.or(index_tail))` where index_tail is `.repeated()` (always succeeds, possibly empty) -> a bare ident becomes Expr::LValue(IndexedLValue{indices:[]}), never Expr::Ident (parser comment at :276 documents exactly this). The real bare-ident lowering is the Expr::LValue empty-indices arm (calls eval_const_ident / eval_shape_ident / resolve_ident / lower_data_ref).

Removed: ast.rs variant (replaced with a TASK-0194 rationale comment) + all 4 dead lower.rs arms (eval_const, eval_shape, lower_index_expr, lower_rvalue). Match exhaustiveness stays valid (variant gone; LValue arms remain). resolve_ident/eval_const_ident/eval_shape_ident still called via the LValue paths -> no dead-code lint. No test referenced Expr::Ident -> no migration.

AC#2 zero-behaviour-change: determinism byte-identical 30/26/0/4 x2 (the proof — unreachable code removed cannot alter valid-input codegen).

GATE: just test 0 failed; e2e 30/26/0/4/0; clippy --all-targets clean (no dead-code/unreachable lint regression elsewhere); ci exit 0. Commit 78f266c.

FEED-FORWARD: removal safe because parser provably never builds it; the LValue empty-indices arm is the canonical bare-ident path — any future "add Expr::Ident back" must also wire a parser construction site or it is dead again.

ORCHESTRATOR review-gate close (phase3-ralph): both reviewers GO, all 4 batch tasks, no NO-GO, no follow-up required (each independently verifiable, not a faked batch-Done). qa-test-runner: workspace 425/0; 3 new tests pass by name; determinism byte-identical x2 + e2e EXACTLY 30/26/0/4/0 + both negatives bite; clippy --all-targets clean; ci exit 0; no new panic on user paths. mped-architect (independently verified, not trusting self-report): TASK-0194 enum-removal AIRTIGHT — Expr::Ident genuinely parser-unreachable (ident_or_call routes bare ident via index_tail .repeated() to Expr::LValue empty-indices, never Expr::Ident; ZERO construction sites repo-wide), only the 4 dead delegating arms removed (the 4 helpers stay LIVE via the surviving LValue arms), NO forced _ wildcard anywhere (clean cargo check = Rust proves exhaustiveness), IrExpr::Ident is a DISTINCT live type untouched; TASK-0193 generalized message literally true for sync+async AND sync,sync AND async,async (no residual overclaim), grammar-sched.md notes 5/7/§5.3 now EXHAUSTIVE + verbatim-matching the shipped message (the recurring comment/doc-lie class NOT repeated; old "both sync and async" §5.3 quote fully removed), variant payload unchanged so no test migration/strength preserved; TASK-0195 genuinely exercises the SYNTHETIC-label NonIntegerShapeExpr path (decl=="<index/loop-bound expression>") asserting Some(span) at the source-recomputed offset (4,14) — closes the TASK-0090 located-vs-position-less boundary both ways; TASK-0197 a genuine control-flow ordering pin (multi-fault dup-worker+unknown-class asserts DuplicateWorker first + !matches UnknownWorkerClass) that WOULD fail if ref-recording moved before the dup guard — constrains the TASK-0196-equivalence invariant, not tautological. decision-0003 upheld (only non-comment src addition is the Display string); scope clean (0193 did NOT widen TASK-0086 option-span; 0194 algo-only no IR/codegen). Per-task Done honest. This cycle converted 4 review-surfaced filed follow-ups into verified Done (good graph hygiene). Task Done stands.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Removed the parser-unreachable algo::ast::Expr::Ident variant and its 4 dead lower.rs match arms.

Unreachability proven: no construction site exists anywhere; parser ident_or_call always routes a bare identifier through index_tail (.repeated(), possibly empty) to Expr::LValue(IndexedLValue{indices:[]}). The real bare-ident path is the Expr::LValue empty-indices arm. Removal chosen over document-as-reserved (cleaner, less dead surface).

Changes:
- algo/ast.rs: Expr::Ident removed; replaced with a rationale comment.
- algo/lower.rs: 4 dead Expr::Ident arms removed (eval_const, eval_shape, lower_index_expr, lower_rvalue), each replaced with a pointer comment to the live LValue arm.

No test referenced the variant; no migration. Zero behaviour change proven by determinism byte-identical x2 (unreachable code).

Gate: just test 0 failed; e2e 30/26/0/4/0; determinism byte-identical 30/26/0/4 x2; clippy --all-targets clean; ci exit 0. Commit 78f266c.
<!-- SECTION:FINAL_SUMMARY:END -->
