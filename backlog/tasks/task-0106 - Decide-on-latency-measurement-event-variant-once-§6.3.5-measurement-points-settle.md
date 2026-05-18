---
id: TASK-0106
title: >-
  Decide on latency/measurement event variant once §6.3.5 measurement points
  settle
status: To Do
assignee: []
created_date: '2026-05-18 01:15'
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
