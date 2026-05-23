---
id: TASK-0111
title: 'ACFG: handle identity-copy dataflow statements'
status: Done
assignee: []
created_date: '2026-05-18 01:23'
updated_date: '2026-05-23 21:11'
labels:
  - compiler
  - ir
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
build_acfg currently skips Dataflow statements whose RHS is not a Call (e.g. identity-copy 'd <-- e'). The link pass also files a parallel limitation (TASK-0097). The right fix for both is co-designed: identity copies should likely become an Operation with no kernel firing but a 'data move' edge, lowered to a Xfer when producer/consumer workers differ. Coordinate with TASK-0097.
<!-- SECTION:DESCRIPTION:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED-until-real-example (orchestrator-direct, cycle 77 sweep). Description: 'build_acfg currently skips Dataflow statements whose RHS is not a Call (e.g. identity-copy d <-- e). The link pass also files a parallel limitation (TASK-0097).' TASK-0097 closed cycle 77 as DEFERRED-until-real-example for the SAME reason: no in-tree example uses identity-copy dataflow syntax. The ACFG and link sides should be co-designed when a real driver surfaces; reopening means filing a single fresh scope-derived task that covers both layers at once (not reopening these two separately). Same deferred-until-trigger pattern as TASK-0097.
<!-- SECTION:FINAL_SUMMARY:END -->
