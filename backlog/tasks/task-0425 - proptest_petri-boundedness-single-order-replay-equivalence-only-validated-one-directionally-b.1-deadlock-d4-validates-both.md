---
id: TASK-0425
title: >-
  proptest_petri: boundedness single-order-replay equivalence only validated
  one-directionally (b.1); deadlock (d4) validates both
status: To Do
assignee: []
created_date: '2026-06-02 02:27'
labels:
  - testing
  - proptest
  - prd-invariant-audit
  - cycle-241
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD-invariant audit (cycle-241) GAP-3. PRD §8.6 claims the single deterministic firing orders replay is equivalent to checking all reachable markings. For DEADLOCK this is fully validated both directions by proptest d4 (proptest_petri.rs:1416, independent exhaustive BFS oracle_can_reach_all_fired). For BOUNDEDNESS, proptest b.1 (proptest_petri.rs:1062) asserts ONLY oracle-clean => pass-accepts; the reverse is deliberately NOT asserted (:1055-1061) because derive_firing_orders single chosen order may legitimately dodge an overflow path the BFS finds. This is HONEST and arguably correct (the reverse genuinely does not hold for an arbitrary single order). FILEABLE IMPROVEMENT (low value, test-completeness): a b5-style property over the SPECIFIC derive_firing_order linearisation: if the BFS finds an overflow reachable AND the derived orders prefix reaches the overflowing marking, the pass MUST flag — i.e. close the gap on the order actually shipped rather than over-claiming the general equivalence. Pointer: tests/proptest_petri.rs b.1 at :1062 + the :1055-1061 one-direction rationale; d4 at :1416 as the both-directions template.
<!-- SECTION:DESCRIPTION:END -->
