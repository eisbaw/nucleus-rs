---
id: TASK-0375
title: >-
  Doc-honesty sweep: grammar-algo.md Atom rule + acfg/build.rs docstring vs the
  now-expressible gather (TASK-0341.03.01 gold-plating)
status: Done
assignee:
  - '@me'
created_date: '2026-05-30 22:46'
updated_date: '2026-05-31 01:11'
labels:
  - docs
  - doc-lie
  - gold-plating
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
GOLD-PLATING follow-up to TASK-0341.03.01. Residual stale doc claims the gather change surfaced (architect P3, gather review), all PRE-EXISTING drift now made live: (1) docs/grammar-algo.md section 1 Atom ::= IntLit | Ident | (AddExpr) understates the accepted language — the parser (algo/parser.rs index_tail) uses the full recursive expr, so Atom effectively includes Ident IndexSuffix* (a nested index). Correct the EBNF + the limitations section to state index position admits a nested data read while const/shape positions do not. (2) acfg/build.rs:411-415 docstring says the algorithm grammar disallows data references in indices — now false (behavior fine, just the comment). (3) TASK-0341.03.01 task description Root-cause-grammar-inspection section repeats the grammar-gap lie (fix at that task close). Recurring-defect-pattern #1.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Doc-only honesty sweep (NO walker/parser code change). (1) docs/grammar-algo.md: add honest note adjacent to the §1 Atom EBNF (lines 100-102) that the parser atom also admits an indexed LValue (nested data read) and a Call via ident_or_call/index_tail, AND add a §6 Limitations item that the gather restriction (data-dependent read accepted in INDEX position, rejected in CONST/SHAPE) is a LOWERING/semantic rule (allow_gather + eval_const returns None for DataRef), NOT grammatical — grammar §1 merges IndexExpr and ConstExpr (parser.rs:514). Cite TASK-0341.03.01. (2) acfg/build.rs:411-415 docstring: fix BOTH stale claims — (a) grammar no longer disallows data refs in indices; (b) "Walking would be a no-op" is now FALSE since collect_dataref_access_expr does NOT recurse into indices, so x[col[k]] does not collect inner col; inert for single-worker (emits from AlgoIR); ref TASK-0373 for distributed. (3) backlog task edit TASK-0341.03.01 --append-notes (single-quoted) clarifying the grammar-gap framing was imprecise; real gap was LOWERING. Gate same as 0374; e2e MUST stay 329/272/0/57/0.
<!-- SECTION:PLAN:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
DONE 2026-05-31 (commit 7d14e8b). Doc-only honesty sweep, no parser/walker behaviour change. (1) docs/grammar-algo.md §1: Atom EBNF now lists CallExpr | LValue (was IntLit|Ident|paren only) reflecting what ident_or_call actually parses; adjacent comment + new §6 limitation item 8 attribute the gather index-vs-const/shape restriction to the SEMANTIC layer (lower_index_expr allow_gather + eval_const returning None for DataRef), NOT the grammar — and note expr_parser MERGES IndexExpr/ConstExpr (parser.rs:514, verified verbatim). (2) acfg/build.rs collect_dataref_access docstring: both stale claims fixed — grammar-disallows-data-refs (now false) and walking-is-a-no-op (now false: collect_dataref_access_expr pushes only the OUTER array, does NOT recurse into indices, so x[col[k]] does not collect col); documented inert-for-single-worker + deferred distributed concern to TASK-0373. (3) TASK-0341.03.01 (Done) appended a clarification note via CLI (single-quoted, words intact). Forward-carried the data_in_access non-collection fact into TASK-0373 notes. VERIFICATION: greps confirmed the only "grammar disallows data references" occurrence was the one fixed; cited symbols (expr_parser docstring, lower_index_expr, allow_gather, eval_const DataRef->None at build.rs:496) all verified in code. Gate green: build/clippy (0 warnings, no doc_lazy_continuation)/test/test-release all 0-fail; e2e 329/272/0/57/0 unchanged. GOTCHA: the TASK-0341.03.01 task FILENAME still encodes the "Atom does not admit nested IndexSuffix" framing, but per workflow it is a Done-task body and was corrected via append-note, not renamed. No new follow-ups filed (TASK-0373 already covers the distributed data_in work).
<!-- SECTION:FINAL_SUMMARY:END -->
