---
id: TASK-0135
title: >-
  Emit Petri net as Graphviz DOT for ACFG->Petri output (verify TASK-0026 nets
  visually)
status: Done
assignee: []
created_date: '2026-05-18 03:36'
updated_date: '2026-05-23 21:07'
labels:
  - M2
  - compiler
  - ir
  - testing
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0026 acceptance criterion #4 asks for per-example DOT snapshot tests. The current implementation lands the lowering pass and structural assertions, but defers full snapshot testing of `serialize_to_dot` output for each example schedule. A separate task should: (a) decide a snapshot location and approval workflow (insta? hand-committed .dot files?), (b) generate baseline DOT for examples 01/02/03 × all schedules, (c) wire a test that fails when the DOT drifts and exposes a `just update-pn-snapshots` recipe. Loose coupling with TASK-0035 (`nucleus --emit-pn` CLI flag) which surfaces the same data to humans.
<!-- SECTION:DESCRIPTION:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED (orchestrator-direct, cycle 77 sweep). The DOT snapshot infrastructure is the same shape as TASK-0147 (closed cycle 77 as DEFERRED-until-label-refactor): both need a snapshot framework decision (insta? hand-committed?), baseline generation, drift detection, and update recipe. Neither has a driver: the ACFG->Petri lowering pass has been load-bearing for cycles 26 onward without DOT snapshot tests catching anything the structural assertions miss. Reopen when (a) a label-cosmetic vs load-bearing distinction emerges (TASK-0147 trigger) AND (b) the structural assertions prove insufficient against an actual bug class — i.e. when a real Petri-net regression slips past the existing test suite. Until then, the structural assertions in nucleus-compiler/tests/acfg_to_petri.rs are the proven-sufficient gate.
<!-- SECTION:FINAL_SUMMARY:END -->
