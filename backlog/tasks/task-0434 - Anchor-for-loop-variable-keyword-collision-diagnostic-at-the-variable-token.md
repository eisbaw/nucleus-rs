---
id: TASK-0434
title: Anchor for-loop-variable keyword-collision diagnostic at the variable token
status: Done
assignee:
  - '@me'
created_date: '2026-06-03 03:44'
updated_date: '2026-06-03 08:09'
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
- [x] #1 for VAR : with a grammar-keyword or Rust-reserved VAR reports a diagnostic anchored at VAR (the variable token), not at the trailing {
- [x] #2 the parity test in algo_parser.rs is updated/replaced to assert the improved anchoring; existing positive for-loops still parse; just ci green
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Impl plan + result (cycle-248): Approach (a) commit-after-for via dedicated for_loop_var() parser in algo/parser.rs. Root cause confirmed empirically: chumsky 0.9 merges alternative errors by furthest input position (Located::max, greater at wins); for_stmt dying at the keyword VAR let the trailing-{ mismatch (further at) win, so the diagnostic pointed at { (line2 col19/20, "found {"). Fix: for_loop_var() captures raw ident chars+span, then_with: valid VAR -> empty().map(SpIdent) (no extra consumption, positive loops byte-identical); reserved VAR -> take_until(just(\{)) to push our error at past the brace, then Simple::custom pinned at the VAR display span. Reject decision+wording factored into shared ident_collision_message() (single source of truth; ident() also routes through it) so for-var diagnostic is byte-identical to data/kernel/worker. New actual diagnostics (verified, deterministic over 2 runs): "for loop : 0..N {}" -> line2 col5 "`loop` cannot be used as a Nucleus identifier: it is a Rust reserved word ... (rename it, e.g. `loop_`)"; "for const : 0..N {}" -> line2 col5 "expected identifier, found keyword `const`". Both anchored at VAR (col5 = right after "for "). Sched sibling (loop VAR : / check loop VAR :) checked empirically: ALREADY correctly anchored (col6 / col12) because the directive is ;-terminated with no competing downstream brace; pinned with new sched_loop_var_keyword_collision_is_anchored_at_the_variable_token test (not changed, just locked). Tests: algo parity test rust_keyword_for_loop_var_is_rejected_with_preexisting_grammar_parity REPLACED by for_loop_var_keyword_collision_is_anchored_at_the_variable_token (asserts col-exact VAR anchoring for both reserved classes + still-rejected + NOT the old "found {" message). docs/grammar-algo.md note-4 caveat updated (was stale/false post-fix). Mega-file gate: for_loop_var pushed parser.rs to 1036 LoC (>1000); reclaimed via shared ident_chars() helper (de-duped 3 raw-ident defs incl sched_directive_hint_stmt local) + docstring trim -> 999 LoC. 1-line margin is tight; filed TASK-0435 to split the token layer into algo/lexical.rs.

IMPLEMENTATION (cycle-251): implementer subagent did the edits but EXHAUSTED ITS TOOL BUDGET mid-CI without committing or reporting. Orchestrator verified the uncommitted work green and committed it as c6c4f10. Fix: dedicated for_loop_var() parser — on a reserved VAR it take_until(just(brace)) so its error out-reaches the trailing block-brace in chumsky 0.9 furthest-position merge, while the DISPLAY span stays var_span (map_one_chumsky_error uses err.span() for line/col, the at-position only drives merge selection — decoupled). Shares the reject decision+wording with ident() via new ident_collision_message() + ident_chars() (SSOT; folds the inline ident_chars in sched_directive_hint_stmt). Sched loop-var positions verified already-correct (semicolon-terminated, no competing downstream token) and pinned. parser.rs at 999/1000 LoC; token-layer split filed TASK-0435.

REVIEW GATE (cycle-251 parallel read-only): qa-test-runner GO + mped-architect GO.
qa NUMBERS (re-run): build OK; clippy clean -D warnings; just test 1285/0/3 dev; just test-release 1283/0/3 (delta 2 = pre-existing TASK-0291); parser tests DETERMINISTIC 3x3 (algo_parser 31/31, sched_parser 38/38 identical each run — chumsky error-path determinism confirmed); just e2e 420/363/0/57/0 UNCHANGED; full just ci EXIT 0 (check-mega-files OK at 999, NOT allow-listed; doc-citation + doc-links fences OK; all 3 negative arms bit correctly). EMPIRICAL diagnostic captured via real nucleus build: "for loop : 0 .. N {" reports at line 2 COLUMN 5 (the VAR `loop`), brace at col 18 — anchored at VAR not the brace.
architect: GO. Mechanism sound (happy-path byte-unchanged via empty().map; display-span=var_span decoupled from merge at-position; .then_with().boxed() pure+idempotent). Determinism preserved (Simple::custom bypasses HashSet expected() rendering via SimpleReason::Custom verbatim). SSOT refactor behaviour-preserving. Sched silent-sibling claim VERIFIED TRUE (const NOT in sched KEYWORDS -> "Rust reserved word" message correct). Both ACs honestly met (BOTH grammar-keyword and Rust-reserved VAR anchor at col-5; parity test REPLACED not deleted with anchoring assertion; positive for-loops still parse).

P1/P2: none. P3 (all folded/filed): (P3-2) brace-less TRUNCATED input degrades the anchor to an EOF "expected brace" (reject still holds, correctness unaffected) — caveat documented in grammar-algo.md note-4 (commit cd1ba50; docstring caveat SKIPPED due to 999-LoC mega-file margin), behavioral fix filed TASK-0434.01. (P3-1) sched ident() is the lone un-deduplicated copy of the new SSOT — folded into TASK-0435. (P3-3) 999/1000 mega-file margin fragile (cheap subset blind to it) — TASK-0435 priority bumped low->medium.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
DONE cycle-251. for-loop-var keyword-collision diagnostic now anchors at the VAR token (empirically line 2 col 5, not the brace). Dedicated for_loop_var() parser via take_until(brace)+var_span display span; shared ident_collision_message()/ident_chars() SSOT. e2e 420/363/0/57/0 unchanged; algo_parser 31/31 + sched_parser 38/38 deterministic 3x; just ci exit 0 (mega-file OK at 999). qa GO + architect GO. Commits c6c4f10 (impl, committed by orchestrator after implementer ran out of budget) + cd1ba50 (P3-2 doc caveat). Follow-ups: TASK-0434.01 (truncated-input anchor), TASK-0435 (token-layer split incl sched-ident dedup, priority->medium).
<!-- SECTION:FINAL_SUMMARY:END -->
