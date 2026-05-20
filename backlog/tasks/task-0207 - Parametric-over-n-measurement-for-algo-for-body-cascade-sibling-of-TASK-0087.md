---
id: TASK-0207
title: >-
  Parametric over-n measurement for algo for{} body cascade (sibling of
  TASK-0087)
status: To Do
assignee: []
created_date: '2026-05-20 05:07'
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
- [ ] #1 Parametric over-n algo-parser fixture iterating n in {0, 1, 2, 5} valid trailing for{}-body statements after a single primary error, asserting the empirically-measured true count for each n, deterministic across two runs, primary-position-stable
- [ ] #2 If algo n-scaling differs from sched n+2, the discrepancy is honestly disclosed in the docstring of algo/parser.rs (the recurring undercount-honesty class — do NOT paper over)
- [ ] #3 Full gate green (just test / clippy --all-targets / e2e 30/26/0/4/0 / determinism byte-identical x2 / negatives bite / ci exit 0); NO parser-logic change
- [ ] #4 When TASK-0199's keyword-anchored sync set fix lands, this parametric assertion flips mechanically from the measured count to == 1 (one-line edit), preserving TASK-0199 AC#7
<!-- AC:END -->
