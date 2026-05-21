---
id: TASK-0219
title: >-
  boundedness::derive_firing_order path-1 is dead code under current pipeline —
  test it or remove it
status: To Do
assignee: []
created_date: '2026-05-21 14:54'
labels:
  - compiler
  - boundedness
  - M4
  - tech-debt
dependencies:
  - TASK-0213
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architecture-review finding (TASK-0213 cycle): the marking-aware logic in derive_firing_order (path 1) is currently never exercised by any in-tree fixture, because path 2 (acfg_to_petri TtoP-arc elision) makes source-order legal on every existing schedule. The implementer characterised path 1 as 'defense-in-depth for nets with softer constraints', but it is currently dead code with no test. Dead-code-with-no-test is a maintenance cost.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Pick ONE: (a) add a synthetic-net unit test that constructs a 2-place net by hand where source-order isn't legal but a legal interleaving exists; assert derive_firing_order discovers it; AND a stuck-state-fallback test asserting check_bounded surfaces the violation via a leftover-trip; OR (b) remove the marking-aware logic and replace it with debug_assert!("source-order is always legal after acfg_to_petri elision").
- [ ] #2 Decision rationale documented in derive_firing_order's docstring or the path-1 module section; recurring-defect (dead code with no test) audit closes this loop.
<!-- AC:END -->
