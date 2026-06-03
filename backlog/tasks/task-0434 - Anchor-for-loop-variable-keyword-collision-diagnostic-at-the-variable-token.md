---
id: TASK-0434
title: Anchor for-loop-variable keyword-collision diagnostic at the variable token
status: To Do
assignee: []
created_date: '2026-06-03 03:44'
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
