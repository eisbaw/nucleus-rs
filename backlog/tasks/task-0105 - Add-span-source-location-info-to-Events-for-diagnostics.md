---
id: TASK-0105
title: Add span/source-location info to Events for diagnostics
status: To Do
assignee: []
created_date: '2026-05-18 01:15'
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
