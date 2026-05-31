---
id: TASK-0375
title: >-
  Doc-honesty sweep: grammar-algo.md Atom rule + acfg/build.rs docstring vs the
  now-expressible gather (TASK-0341.03.01 gold-plating)
status: In Progress
assignee:
  - '@me'
created_date: '2026-05-30 22:46'
updated_date: '2026-05-31 00:52'
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
