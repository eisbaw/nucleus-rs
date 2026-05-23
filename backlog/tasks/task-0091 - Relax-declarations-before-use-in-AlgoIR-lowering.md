---
id: TASK-0091
title: Relax declarations-before-use in AlgoIR lowering
status: Done
assignee: []
created_date: '2026-05-18 00:25'
updated_date: '2026-05-23 20:49'
labels:
  - M0
  - compiler
  - ir
  - follow-up
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0009 enforces declarations-before-use by lowering items in source order. If a real example needs forward references between consts/data/kernels, switch to a two-pass lowering: collect declarations first, then evaluate. Out of scope until a driving example needs it.
<!-- SECTION:DESCRIPTION:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED (orchestrator-direct, cycle 77). The task description explicitly says 'Out of scope until a driving example needs it' — no real example in tier-1 needs forward references between consts/data/kernels. Reopen when an example genuinely requires forward decl-before-use (mirror the TASK-0144.03 / TASK-0144.02 deferred-closure pattern: file a fresh scope-derived task at trigger time rather than carry a To-Do indefinitely).
<!-- SECTION:FINAL_SUMMARY:END -->
