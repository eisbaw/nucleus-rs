---
id: TASK-0133
title: >-
  Iteration encoding optimisation: parametrise static repeat instead of
  unrolling in ACFG->Petri
status: Done
assignee: []
created_date: '2026-05-18 03:36'
updated_date: '2026-05-23 21:08'
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

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED (orchestrator-direct, cycle 77 sweep). Description: 'Investigate after analyses land and there is measurable evidence the unrolled net is the bottleneck.' Today no contributor has measured the unrolled-net cost as a bottleneck — every-cell e2e + determinism gates run in seconds across the full 88-cell matrix. The N>>1k iteration ranges the parametric encoding would help with (e.g. real CNN training, audio-streaming) are not in v2's example surface. Reopen on measured evidence of the bottleneck (cargo build time dominated by petri_to_events on a specific schedule). Same deferred-closure pattern.
<!-- SECTION:FINAL_SUMMARY:END -->
