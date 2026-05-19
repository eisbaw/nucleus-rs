---
id: TASK-0195
title: >-
  Pin that synthetic NonIntegerShapeExpr carries Some(span) — the
  located-vs-position-less boundary
status: To Do
assignee: []
created_date: '2026-05-19 16:13'
updated_date: '2026-05-19 16:13'
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
- [ ] #1 A positive test feeds a program whose loop-bound/index expression is non-integer (triggering the synthetic NonIntegerShapeExpr) and asserts the LowerError carries Some(span) at the CORRECT offset (validated via error::offset_to_line_col against the crafted source), NOT None
- [ ] #2 Full gate green (just test/e2e 30/26/0/4/0/determinism byte-identical/clippy --all-targets/ci); no behaviour change
<!-- AC:END -->
