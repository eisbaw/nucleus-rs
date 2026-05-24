---
id: TASK-0285
title: Prune or use TraceCapture (TASK-0280 follow-up)
status: To Do
assignee: []
created_date: '2026-05-24 17:32'
labels:
  - infra
  - tooling
  - dead-code
  - follow-up
  - TASK-0280
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0280 cycle 108 audit found the TraceCapture RAII guard + TRACE_SINK thread-local + test_sink_active helper in nucleus/nucleus-compiler/src/trace.rs are referenced ONLY inside that file. No test in the workspace calls TraceCapture::start; no in-source consumer of trace::emit() outside the nuc_trace! macro. The subsystem exists as a future-test escape hatch but has had zero uses since landing.

## Options
1. PRUNE: remove TraceCapture struct + TRACE_SINK + test_sink_active (~30 LoC). The nuc_trace! macro simplifies to a single trace_enabled() check. Future tests that need to capture trace lines re-introduce the sink. Smaller surface area.
2. USE: write a test that pins a known trace emission point (e.g. the halo_inference advisory at driver/src/main.rs:399) using TraceCapture. Validates the test sink is wired correctly and gives TraceCapture a real anchor.
3. KEEP: leave as-is with the TASK-0280 module-doc note explaining its preserved-as-convention status. Lowest churn but the dead-code remains.

## Recommendation
Option 1 (PRUNE). The driver-level nuc_trace! caller can be tested via stderr scraping if a test ever needs it (this is the same approach the 4 e2e cells use today for their PASS/FAIL banners). The TraceCapture pattern adds complexity (thread_local + RAII + sentinel) that no test ever exercises — pruning is honest dead-code removal.

## Acceptance
- TraceCapture struct removed.
- TRACE_SINK thread_local removed.
- test_sink_active() removed.
- nuc_trace! macro simplifies to a single trace_enabled() check.
- trace.rs module doc updated to remove the 'Known dead code' section.
- All tests pass (no test depends on these symbols today).

## Honest scope
This is dead-code hygiene. The codebase is correct without it; the prune is purely surface-area reduction. If a future test genuinely needs trace capture, the TraceCapture pattern can be re-introduced as a test-only helper.

## Dependencies
- TASK-0280 (Done; decision made, dead code identified).
<!-- SECTION:DESCRIPTION:END -->
