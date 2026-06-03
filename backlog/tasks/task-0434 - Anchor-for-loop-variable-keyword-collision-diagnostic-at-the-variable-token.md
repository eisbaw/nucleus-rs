---
id: TASK-0434
title: Anchor for-loop-variable keyword-collision diagnostic at the variable token
status: In Progress
assignee:
  - '@me'
created_date: '2026-06-03 03:44'
updated_date: '2026-06-03 06:56'
labels:
  - compiler
  - frontend
  - diagnostics
  - cycle-248
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0433 follow-up: the algo for-loop VARIABLE position (`for VAR : lo .. hi { }`) does NOT anchor a keyword-collision diagnostic at VAR. When VAR is a grammar keyword OR a Rust-reserved keyword (RUST_RESERVED), ident() rejects it (so the var is never admitted to the AST and never reaches codegen — AC#1 of TASK-0433 holds), but chumsky 0.9's error-merge surfaces the more-consuming downstream { -mismatch error instead of the ident() custom message. Result: the diagnostic points at the { not the offending VAR. This is PRE-EXISTING (grammar keywords in the same position behave identically) and pinned by tests/algo_parser.rs::rust_keyword_for_loop_var_is_rejected_with_preexisting_grammar_parity. Fix likely needs a dedicated for-var ident parser that does not backtrack the whole for_stmt branch (e.g. commit after the for keyword + a non-recovering ident, or map the merged error). LOW priority — correctness (never-reach-codegen) is already guaranteed; this is diagnostic quality only.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 for VAR : with a grammar-keyword or Rust-reserved VAR reports a diagnostic anchored at VAR (the variable token), not at the trailing {
- [ ] #2 the parity test in algo_parser.rs is updated/replaced to assert the improved anchoring; existing positive for-loops still parse; just ci green
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Impl plan + result (cycle-248): Approach (a) commit-after-for via dedicated for_loop_var() parser in algo/parser.rs. Root cause confirmed empirically: chumsky 0.9 merges alternative errors by furthest input position (Located::max, greater at wins); for_stmt dying at the keyword VAR let the trailing-{ mismatch (further at) win, so the diagnostic pointed at { (line2 col19/20, "found {"). Fix: for_loop_var() captures raw ident chars+span, then_with: valid VAR -> empty().map(SpIdent) (no extra consumption, positive loops byte-identical); reserved VAR -> take_until(just(\{)) to push our error at past the brace, then Simple::custom pinned at the VAR display span. Reject decision+wording factored into shared ident_collision_message() (single source of truth; ident() also routes through it) so for-var diagnostic is byte-identical to data/kernel/worker. New actual diagnostics (verified, deterministic over 2 runs): "for loop : 0..N {}" -> line2 col5 "`loop` cannot be used as a Nucleus identifier: it is a Rust reserved word ... (rename it, e.g. `loop_`)"; "for const : 0..N {}" -> line2 col5 "expected identifier, found keyword `const`". Both anchored at VAR (col5 = right after "for "). Sched sibling (loop VAR : / check loop VAR :) checked empirically: ALREADY correctly anchored (col6 / col12) because the directive is ;-terminated with no competing downstream brace; pinned with new sched_loop_var_keyword_collision_is_anchored_at_the_variable_token test (not changed, just locked). Tests: algo parity test rust_keyword_for_loop_var_is_rejected_with_preexisting_grammar_parity REPLACED by for_loop_var_keyword_collision_is_anchored_at_the_variable_token (asserts col-exact VAR anchoring for both reserved classes + still-rejected + NOT the old "found {" message). docs/grammar-algo.md note-4 caveat updated (was stale/false post-fix). Mega-file gate: for_loop_var pushed parser.rs to 1036 LoC (>1000); reclaimed via shared ident_chars() helper (de-duped 3 raw-ident defs incl sched_directive_hint_stmt local) + docstring trim -> 999 LoC. 1-line margin is tight; filed TASK-0435 to split the token layer into algo/lexical.rs.
<!-- SECTION:NOTES:END -->
