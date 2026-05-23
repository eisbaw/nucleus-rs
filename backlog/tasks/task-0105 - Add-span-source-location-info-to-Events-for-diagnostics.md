---
id: TASK-0105
title: Add span/source-location info to Events for diagnostics
status: Done
assignee: []
created_date: '2026-05-18 01:15'
updated_date: '2026-05-23 21:13'
labels:
  - M2
  - events
  - diagnostics
  - followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0015 left Events without spans. Errors and warnings that want to point at the algorithm or schedule source line that produced an event need either a parallel SpanList or a wrapping struct. Decision deferred until first diagnostic actually needs it. Reference: PRD §8.3, TASK-0015 notes.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Each Event carries (or has a sidecar mapping to) the AlgoIR/SchedIR statement span it was projected from;diagnostic messages quote the originating source line
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED-until-first-diagnostic-needs-it (orchestrator-direct, cycle 77 sweep). The task description explicitly says: 'Decision deferred until first diagnostic actually needs it.' Today: 0 diagnostics need event-level spans (LinkError carries source span via TASK-0099; LowerError carries source span via TASK-0090; sched-lower errors carry source span). Events themselves are an internal IR layer that downstream backends consume; user-facing diagnostics come from the source-level passes that have spans already. Reopen at the first concrete diagnostic-need (e.g. 'this Push deadlocked because of <source line>'). Same deferred-until-trigger pattern as the cycle-77 sweep.
<!-- SECTION:FINAL_SUMMARY:END -->
