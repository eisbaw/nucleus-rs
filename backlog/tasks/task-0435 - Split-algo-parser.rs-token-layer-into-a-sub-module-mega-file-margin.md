---
id: TASK-0435
title: Split algo/parser.rs token-layer into a sub-module (mega-file margin)
status: To Do
assignee: []
created_date: '2026-06-03 06:35'
updated_date: '2026-06-03 08:07'
labels:
  - compiler
  - frontend
  - refactor
  - tech-debt
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
algo/parser.rs sits at 999 LoC (1-line margin under the 1000 mega-file gate) after TASK-0434 added the for_loop_var dedicated parser. The token/lexical layer (comment_or_ws, pad, padded_spanned, ident_chars, ident_collision_message, ident, for_loop_var, int_lit, scalar_type, keyword) is a cohesive seam that can extract to algo/lexical.rs, dropping parser.rs well under the limit and giving headroom. Preferred over an allow-list entry (per memory feedback-cheap-subset-blind-to-structural-fences: split, do not allow-list). Requires sharing KEYWORDS + a few consts with the new module (pub(super)). LOW priority structural hygiene; no behaviour change.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0434 review (cycle-251 architect P3-1 + P3-3): (P3-3) PRIORITY bumped low->medium — parser.rs sits at 999/1000 LoC (1-line margin under the mega-file fence the CHEAP pre-commit subset is BLIND to, per feedback-cheap-subset-blind-to-structural-fences); the next unrelated docstring/helper line in this file flips just ci RED. Prioritise the token-layer split to restore headroom. (P3-1) WHILE splitting, ALSO de-duplicate the sched-side reject logic: sched/parser.rs::ident still carries the byte-identical inline two-message reject (KEYWORDS-first then is_rust_reserved -> collision_message) + its own inline ident-chars closure (sched/parser.rs ~210/215-233). TASK-0434 created algo/parser.rs::ident_collision_message + ident_chars as the SINGLE SOURCE OF TRUTH but only wired the ALGO parser through it; the sched ident() is now the lone un-deduplicated copy. If ident_collision_message ever gains a third reserved class, sched silently diverges (silent-sibling risk). Lift ident_collision_message + ident_chars to a shared spot (e.g. the new algo/lexical.rs or a crate-level reserved/lexical helper) and route BOTH algo and sched ident() through it.
<!-- SECTION:NOTES:END -->
