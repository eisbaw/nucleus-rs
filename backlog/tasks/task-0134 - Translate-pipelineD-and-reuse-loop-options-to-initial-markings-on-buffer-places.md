---
id: TASK-0134
title: >-
  Translate pipeline=D and reuse loop options to initial markings on buffer
  places
status: To Do
assignee: []
created_date: '2026-05-18 03:36'
labels:
  - M2
  - M4
  - compiler
  - ir
  - scheduling
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD §8.2 maps "pipeline depth / latency-hiding head-start" to initial markings on places. The `acfg_to_petri` pass landing under TASK-0026 currently sets every buffer place's `initial_marking` to 0. This is correct for sync/async transfers but does not realise the pipeline/reuse story. Prerequisites: the schedule grammar carries `loop X : pipeline=D` (already lowered in SchedIR — verify with sched_parser tests), and ACFG `XferPlaceholder`/`Operation` carries enough context to know which buffer place a given pipeline=D refers to (may need a new ACFG annotation pass). After that, the `acfg_to_petri` builder pre-marks the buffer place's initial_marking to D.
<!-- SECTION:DESCRIPTION:END -->
