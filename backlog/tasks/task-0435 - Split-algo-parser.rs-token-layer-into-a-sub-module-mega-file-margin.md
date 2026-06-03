---
id: TASK-0435
title: Split algo/parser.rs token-layer into a sub-module (mega-file margin)
status: To Do
assignee: []
created_date: '2026-06-03 06:35'
labels:
  - compiler
  - frontend
  - refactor
  - tech-debt
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
algo/parser.rs sits at 999 LoC (1-line margin under the 1000 mega-file gate) after TASK-0434 added the for_loop_var dedicated parser. The token/lexical layer (comment_or_ws, pad, padded_spanned, ident_chars, ident_collision_message, ident, for_loop_var, int_lit, scalar_type, keyword) is a cohesive seam that can extract to algo/lexical.rs, dropping parser.rs well under the limit and giving headroom. Preferred over an allow-list entry (per memory feedback-cheap-subset-blind-to-structural-fences: split, do not allow-list). Requires sharing KEYWORDS + a few consts with the new module (pub(super)). LOW priority structural hygiene; no behaviour change.
<!-- SECTION:DESCRIPTION:END -->
