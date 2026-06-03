---
id: TASK-0435
title: Split algo/parser.rs token-layer into a sub-module (mega-file margin)
status: Done
assignee:
  - Mark Ruvald Pedersen
created_date: '2026-06-03 06:35'
updated_date: '2026-06-03 09:19'
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

Cycle-252 DONE. Split landed (commit 68efa22): algo/parser.rs 999->809 LoC (191 under the fence) by extracting the token layer to new algo/tokens.rs (pub(super)); shared ident_chars + ident_collision_message(s, keywords) lifted to new crate-level src/lexical.rs. Both algo::tokens::ident/for_loop_var and sched::parser::ident now route through it; KEYWORDS set is a parameter (algo vs sched differ) so diagnostics stay byte-identical. P3-1 de-dup COMPLETE: the lone un-deduplicated sched copy is retired. GATES: just test 1289 pass / 0 fail (+4 new lexical unit tests pinning KEYWORDS-first ordering + parameterization); test-release 1287/0; e2e 420/363/0/57/0 unchanged (2 runs, non-flake); clippy/build/check-mega-files/check-doc-links green. Review: qa GO + architect GO (no P1/P2; behavioural equivalence of sched ident() confirmed byte-identical at code layer; de-dup sound; tests bite). GOTCHA (forward-carried): check-doc-links runs cargo doc WITHOUT --document-private-items, so it is BLIND to broken intra-doc links on PRIVATE fns - moving private fns between modules can silently break their [`link`]s. Verify with RUSTDOCFLAGS=-D rustdoc::broken_intra_doc_links cargo doc --document-private-items. One genuinely-broken cross-module link ([`super::parser::sched_directive_hint_stmt`]) + a cross-crate [`take_until`] link were demoted to plain code spans. See memory feedback-visibility-tighten-doclink-trap.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Split algo/parser.rs token-layer into algo/tokens.rs and lifted the shared identifier token-shape + reserved-word reject decision to crate-level src/lexical.rs (ident_chars + parameterized ident_collision_message). Restored mega-file headroom (999->809 LoC) and retired the silent-sibling risk of the duplicated sched ident() reject (P3-1). Pure refactor, byte-identical diagnostics and e2e (420/363/0/57/0). qa GO + architect GO.
<!-- SECTION:FINAL_SUMMARY:END -->
