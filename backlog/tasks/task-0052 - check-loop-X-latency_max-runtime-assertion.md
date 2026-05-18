---
id: TASK-0052
title: 'check loop X : latency_max runtime assertion'
status: To Do
assignee: []
created_date: '2026-05-17 23:09'
labels:
  - language
  - compiler
  - tooling
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Implement the runtime assertion machinery from PRD §6.3.5. Compiler injects timing instrumentation at iteration boundaries; backend lowers to the appropriate clock + violation action.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Schedule parser accepts 'check loop VAR : latency_max=T, on_violation=panic|log|count'.
- [ ] #2 compiler instruments the loop body with timer start/end; emits comparison against threshold; emits violation handler per on_violation choice.
- [ ] #3 Tier 1 uses std::time::Instant; tier 2 MPI_Wtime; tier 3 backend-specified monotonic clock.
- [ ] #4 Test: a deliberately slow kernel triggers latency_max=panic; the program panics.
- [ ] #5 Test: on_violation=count tallies and reports at exit.
- [ ] #6 Test: example 14's embedded_multimcu schedule's check survives the full pipeline.
- [ ] #7 Implementation notes record design questions (e.g. end-to-end-latency across pipeline depth not supported; how to communicate this in error messages).
- [ ] #8 Implementation notes record honest limitations (checkable only, not prescriptive; clock resolution varies by backend).
<!-- AC:END -->
