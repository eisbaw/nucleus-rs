---
id: TASK-0106
title: >-
  Decide on latency/measurement event variant once §6.3.5 measurement points
  settle
status: Done
assignee: []
created_date: '2026-05-18 01:15'
updated_date: '2026-05-23 21:06'
labels:
  - M2
  - events
  - measurement
  - followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD §6.3.5 talks about check directive measurement points but the measurement model is TBD. When it settles, decide whether to add an Event::Latency variant or thread measurement through a sidecar. Reference: PRD §6.3.5, §8.3, TASK-0015 notes.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Measurement model in §6.3.5 settled and either an Event variant or a sidecar mechanism is implemented
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED (orchestrator-direct, cycle 77 sweep). Description: 'When it settles, decide whether to add an Event::Latency variant or thread measurement through a sidecar.' The TASK-0052 family (cycles ~52-55, on_violation=panic/log/count) landed without needing an Event::Latency variant — measurement happens at the codegen layer via CheckFrame (per-loop std::time::Instant + Drop guard). PRD §6.3.5 has substantially settled into that shape. If a deeper measurement model (e.g. histogram emission, cross-loop latency budgets) ever needs a first-class Event variant, file a fresh task scoped to that need at the time. Same deferred-closure pattern as the cycle-77 sweep.
<!-- SECTION:FINAL_SUMMARY:END -->
