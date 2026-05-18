---
id: TASK-0133
title: >-
  Iteration encoding optimisation: parametrise static repeat instead of
  unrolling in ACFG->Petri
status: To Do
assignee: []
created_date: '2026-05-18 03:36'
labels:
  - M2
  - compiler
  - ir
  - optimisation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Today ACFG `Repeat { range }` is unrolled into N copies of body transitions in `acfg_to_petri` (TASK-0026 implementation). For ranges of length >>1k this explodes the place/transition count linearly. Alternative: encode a static repeat as a parametric firing — one set of body transitions plus a loop-counter place pre-marked with N tokens that gates iteration count. Downstream EventList projection (TASK-0027) and analysis passes (TASK-0028/0029) need to agree on the encoding because the net topology changes. Investigate after analyses land and there is measurable evidence the unrolled net is the bottleneck.
<!-- SECTION:DESCRIPTION:END -->
