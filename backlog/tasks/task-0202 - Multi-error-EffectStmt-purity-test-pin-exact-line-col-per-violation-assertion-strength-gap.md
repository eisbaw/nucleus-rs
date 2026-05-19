---
id: TASK-0202
title: >-
  Multi-error EffectStmt purity test: pin exact (line, col) per violation
  (assertion-strength gap)
status: To Do
assignee: []
created_date: '2026-05-19 22:32'
labels:
  - compiler
  - diagnostics
  - tests
  - follow-up
  - M0
dependencies:
  - TASK-0089
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
compiler/tests/algo_lower.rs::multiple_effect_purity_violations_each_reported pins .len()==2, the two callee names, and spans[0] != spans[1]. It does NOT pin the exact (line, col) of each violation via offset_to_line_col — only the singular-violation test located_effect_purity_error_has_correct_line_col does. A regression placing the multi-violation spans at distinct-but-wrong positions (the wrong-token-on-the-line bug class) would slip through. Filed from TASK-0089 architecture review (Finding #3, 2026-05-20).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Extend multiple_effect_purity_violations_each_reported to compute expected (line, col) for each callee occurrence via offset_to_line_col and assert each pair (analogous to located_effect_purity_error_has_correct_line_col but for the multi-error path); negative-fixture is the two-effect-stmt-to-pure-kernel pattern; both line:col pairs pinned with no len-only blanket assertion smell
- [ ] #2 just test passes; just ci exits 0; no behaviour change for valid input (e2e 30/26/0/4/0; determinism-check byte-identical x2); clippy --workspace --all-targets clean
<!-- AC:END -->
