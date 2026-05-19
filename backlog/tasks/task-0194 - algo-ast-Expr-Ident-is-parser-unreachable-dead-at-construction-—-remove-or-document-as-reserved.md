---
id: TASK-0194
title: >-
  algo::ast::Expr::Ident is parser-unreachable dead-at-construction — remove or
  document as reserved
status: To Do
assignee: []
created_date: '2026-05-19 15:48'
updated_date: '2026-05-19 15:48'
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
- [ ] #1 Expr::Ident is either removed (with its now-dead lower.rs handling) OR documented in ast.rs as an intentional reserved variant with rationale; the parser-unreachability is verified (grep/test)
- [ ] #2 Zero behaviour change: just test green, e2e 30/26/0/4/0, determinism byte-identical, clippy --all-targets clean, ci exit 0
<!-- AC:END -->
