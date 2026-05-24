---
id: TASK-0285
title: Prune or use TraceCapture (TASK-0280 follow-up)
status: Done
assignee:
  - '@mped-orchestrator'
created_date: '2026-05-24 17:32'
updated_date: '2026-05-24 17:38'
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## CYCLE-109 LANDING (orchestrator-led, 2026-05-24)

TASK-0285 closed. Pruned the unused TraceCapture machinery (option 1 PRUNE per the task brief recommendation).

### Removed

- TraceCapture struct (RAII guard).
- TraceCapture::start() + TraceCapture::lines() + Drop impl.
- TRACE_SINK thread_local (RefCell<Option<Vec<String>>>).
- test_sink_active() helper.
- The 'captured' branch of emit() (just stderr now if trace_enabled).
- 'std::cell::RefCell' import (no longer needed).
- The 'use std::cell::RefCell' line.
- The test-sink-active check in the nuc_trace! macro (now just trace_enabled()).

### Kept

- nuc_trace! macro itself (production caller at driver/main.rs:399).
- trace_enabled() function.
- emit() function (now just the stderr write).
- Module-level docs (updated to reflect cycle-109 prune; previous 'Known dead code' section folded into the Decision block, now describes the cycle-109 removal).

### Diff metrics

- nucleus/nucleus-compiler/src/trace.rs: -86 lines, +20 lines (net -66).
- Total module shrink: ~50% reduction.

### Gate post-prune

- cargo test --workspace: 818 / 0 / 3 (unchanged; no test depended on the removed symbols, as expected — that was the reason to prune).
- cargo clippy --workspace --all-targets -- -D warnings: clean.
- e2e + determinism not re-run (no codegen path touched; the production nuc_trace! emission shape is identical).

### Honest scope

This was dead-code hygiene with zero codebase touchpoint outside trace.rs itself. The removal is reversible — if a future test needs trace capture, the RefCell<Option<Vec<String>>> + RAII guard pattern is ~30 LoC to re-introduce. Per the cycle-108 module-doc commentary, the prune is documented so the next person revisiting this facility makes the re-introduce-or-stderr-scrape call deliberately.

### ACs MET

- TraceCapture struct removed: MET.
- TRACE_SINK thread_local removed: MET.
- test_sink_active() removed: MET.
- nuc_trace! macro simplifies to single trace_enabled() check: MET.
- trace.rs module doc updated to remove 'Known dead code' section: MET (folded into the cycle-109 prune note in the Decision block).
- All tests pass: MET (818/0/3 unchanged).

Status: Done.
<!-- SECTION:NOTES:END -->
