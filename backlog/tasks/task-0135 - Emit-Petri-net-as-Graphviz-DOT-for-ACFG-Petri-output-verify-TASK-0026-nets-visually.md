---
id: TASK-0135
title: >-
  Emit Petri net as Graphviz DOT for ACFG->Petri output (verify TASK-0026 nets
  visually)
status: To Do
assignee: []
created_date: '2026-05-18 03:36'
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
