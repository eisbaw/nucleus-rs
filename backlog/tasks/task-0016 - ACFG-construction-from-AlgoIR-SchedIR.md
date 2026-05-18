---
id: TASK-0016
title: 'ACFG construction from (AlgoIR, SchedIR)'
status: To Do
assignee: []
created_date: '2026-05-17 23:04'
labels:
  - M1
  - compiler
  - ir
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Build the application control-flow graph that drives subsequent passes. Nodes: operation, repeat, sync (placeholder), xfer (placeholder). Tree shape per the 2013 thesis simplification.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 compiler crate exposes build_acfg(LinkedIR) -> ACFG.
- [ ] #2 Top-level statements become a chain of acfg nodes; for-loops become acfg::repeat with body subtree.
- [ ] #3 Each operation node carries its placement (worker id) and the DAG of operations within the basic block.
- [ ] #4 Test: snapshot tests for ACFG output on each example after linking.
- [ ] #5 Implementation notes record design questions (graph vs tree representation, when to switch to graph for back-edges if needed).
- [ ] #6 Implementation notes record honest limitations (no support for if-statements yet; only for-loops and dataflow).
<!-- AC:END -->
