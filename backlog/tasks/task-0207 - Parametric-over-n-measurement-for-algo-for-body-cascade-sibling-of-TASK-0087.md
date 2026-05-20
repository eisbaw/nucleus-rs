---
id: TASK-0207
title: >-
  Parametric over-n measurement for algo for{} body cascade (sibling of
  TASK-0087)
status: Done
assignee:
  - '@mped'
created_date: '2026-05-20 05:07'
updated_date: '2026-05-20 17:33'
labels:
  - compiler
  - language
  - follow-up
  - M0
dependencies:
  - TASK-0199
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0087 close-out cycle landed parametric over-n fixtures for the SCHED-parser worker_class/memory_region brace-body n+2 cascade (nested_brace_body_error_surfaces_n_plus_two_parametric_{worker_class,memory_region}, n in {0, 1, 2, 5}). The ALGO parser has the analogous for{} body case — sched/parser.rs:797 explicitly notes "sched analog of the algorithm for { ... }-body nested-; shape" — and per TASK-0199 description it was "corrected once from 'one' to 'two'; may itself scale similarly — re-measure". The algo side is currently only pinned at a single n; the masking-defect class is open there until measured parametrically. Add the algo counterpart fixture iterating n in {0, 1, 2, 5} valid trailing for{}-body statements after a single primary error, assert errors().len() == measured-true-count, deterministic, primary-position-stable. If algo also scales as n+2, document so; if it differs from the sched side, file the discrepancy honestly (separate root cause). When TASK-0199 lands the keyword-anchored sync set fix, this assertion also flips to == 1 (TASK-0199 AC#7 — "a single error inside the algo for{} body collapses to PRIMARY only, pinned by PARAMETRIZED regression tests over brace-body field count n"). Doc+test+honesty only, NO parser-logic change here either. Reference template: nucleus/compiler/tests/sched_parser.rs::nested_brace_body_error_surfaces_n_plus_two_parametric_worker_class (commit 76a914d). Same shape applied to algo.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Parametric over-n algo-parser fixture iterating n in {0, 1, 2, 5} valid trailing for{}-body statements after a single primary error, asserting the empirically-measured true count for each n, deterministic across two runs, primary-position-stable
- [x] #2 If algo n-scaling differs from sched n+2, the discrepancy is honestly disclosed in the docstring of algo/parser.rs (the recurring undercount-honesty class — do NOT paper over)
- [x] #3 Full gate green (just test / clippy --all-targets / e2e 30/26/0/4/0 / determinism byte-identical x2 / negatives bite / ci exit 0); NO parser-logic change
- [x] #4 When TASK-0199's keyword-anchored sync set fix lands, this parametric assertion flips mechanically from the measured count to == 1 (one-line edit), preserving TASK-0199 AC#7
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
TASK-0207 (commit 253e1b0): parametric over-n algo for-body fixture lands. EMPIRICAL FINDING: algo for{}-body cascade is the CONSTANT 2 (NOT n+2). Measured deterministic across two runs at n in {0, 1, 2, 5} (fixture) and out-of-fixture {3, 8, 12} (probe): always exactly 2 errors — primary at L5C14 + structural Unexpected at line (6+n) column 1 (the body-close } line). DIVERGES from sched (n+2 brace-body cascade); structural root cause is that the algo for-body uses a bare stmt.clone().repeated() with NO inner per-; recovery layer, whereas sched worker_class/memory_region bodies have field.recover_with(skip_until(...)).repeated() producing one cascade entry per inner ;. When TASK-0199 lands both fixtures collapse mechanically to == 1 (same one-line edit). Comprehensive doc-lie sweep applied per cycle-5-review lesson — 4 carriers updated: (a) algo/parser.rs module-doc Known limitations, (b) algo/parser.rs program_parser inline ref, (c) sched/parser.rs program_parser sched-analog paragraph, (d) sched_parser.rs nested_brace_body_error_surfaces_bounded_follow_ons_* docstring. AC#1 met (fixture iterates n in {0,1,2,5}, asserts measured count, deterministic, primary L5C14 stable); AC#2 met (divergence honestly disclosed in algo/parser.rs module-doc + cross-file carriers); AC#3 met (gate green: 459 tests, clippy clean, e2e 30/26/0/4/0, determinism x2 byte-identical, both negatives bite, ci exit 0); AC#4 met (the expected literal == 2 is a one-line edit to == 1 when TASK-0199 lands). Gotchas: (i) the algo case never had a single-n disclosure pin in algo_parser.rs so no pre-existing test needed rebasing; (ii) the existing sched_parser.rs:36 reference to the algo side is now accurate since the algo module-doc was rewritten — left unchanged; (iii) backlog task md files for TASK-0080/0087 were left untouched per the CLAUDE.md rule (only the backlog CLI mutates task notes); their old text referring to algo as having +1/+2 follow-ons remains consistent with constant-2 = primary + 1 follow-on.

CORRECTION (review-gate cycle, 2026-05-20): the prior close-out paragraph's mechanism explanation ("sched worker_class/memory_region bodies have field.recover_with(skip_until(...)).repeated() producing one cascade entry per inner ;") is FACTUALLY WRONG. Verified by reading the actual parser code: sched/parser.rs:393 and :450 use BARE field.repeated() with NO inner recover_with; the only .recover_with site in sched/parser.rs is at line 841 — at the DIRECTIVE level, not the field level. Both parsers have only ONE recovery site at the outer item/directive boundary. The TRUE structural reason for the algo-vs-sched divergence: sched's top-level directive grammar accepts only directive-keyword-led items, so each residual field-keyword line re-fails directive parsing and re-triggers the directive-level recover_with → one cascade per residual ;; algo's top-level grammar accepts Stmt items, so residual x[i] <-- inc(i); lines parse cleanly as top-level Items with zero re-failures. The empirical CONSTANT-2 measurement, the AC#1-AC#4 closure, and the post-TASK-0199 flip prediction are ALL UNCHANGED — only the mechanism explanation was wrong (the classic recurring comment-doc-lie class). Comprehensive sweep applied across 6 carriers found by codebase grep (this note, TASK-0199 cross-layer measurement, algo/parser.rs Known-limitations, algo_parser.rs fixture rustdoc, sched_parser.rs nested_brace_body docstring, sched_parser.rs inline comment at n=1 fixture).
<!-- SECTION:NOTES:END -->
