---
id: TASK-0375
title: >-
  Doc-honesty sweep: grammar-algo.md Atom rule + acfg/build.rs docstring vs the
  now-expressible gather (TASK-0341.03.01 gold-plating)
status: To Do
assignee: []
created_date: '2026-05-30 22:46'
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
