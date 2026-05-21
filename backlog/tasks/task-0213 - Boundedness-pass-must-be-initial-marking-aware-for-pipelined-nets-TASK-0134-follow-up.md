---
id: TASK-0213
title: >-
  Boundedness pass must be initial-marking-aware for pipelined nets (TASK-0134
  follow-up)
status: To Do
assignee: []
created_date: '2026-05-21 13:41'
labels:
  - compiler
  - ir
  - scheduling
dependencies:
  - TASK-0134
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0134 lands initial_marking = D on cross-worker buffer places inside pipeline=D loops. The current boundedness check uses derive_firing_order = source-order; with initial_marking=D and capacity=N=D, the first Push tries to deposit one more token (D+1) and overflows because the buffer is already full at startup.

This is a structural tension: interpretation (a) of PRD §8.2 puts D head-start tokens in the buffer place, which is incompatible with D = N capacity under a source-order firing trace.

Two viable resolutions, each documented in TASK-0134 notes:
1. Generalise derive_firing_order to be marking-aware. With initial_marking > 0 on a buffer place, the consumer (Wait) should fire before the producer (Push) for the first marking-many iterations. This requires examining the initial marking and reordering Push/Wait pairs at the boundedness-input stage.
2. Change the IR encoding: when pipeline=D applies, eliminate D producer firings from the unrolled Repeat body (representing them as pre-fired by the initial marking). This is a structural acfg_to_petri change.

AC#5 of TASK-0134 explicitly requires boundedness/deadlock to pass on a pipelined fixture. This task delivers that.

Acceptance criteria
- #1 derive_firing_order OR acfg_to_petri updated so a pipeline=D, buffer=D, body-with-Push-Wait net passes check_bounded.
- #2 The deadlock check also passes for the same fixture.
- #3 Existing non-pipelined fixtures regress unchanged (every example without pipeline= still fires in source order).
- #4 Determinism preserved (BTreeMap-driven, no HashMap).
- #5 Add the assertion currently dropped from acfg_to_petri.rs's e2e_example_13_pipeline_parallel_passes_boundedness_and_deadlock test back into the test suite.
<!-- SECTION:DESCRIPTION:END -->
