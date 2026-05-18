---
id: TASK-0044
title: M6 — Full tier-1 backend matrix
status: To Do
assignee: []
created_date: '2026-05-17 23:08'
labels:
  - M6
  - validation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Tier-1 milestone: openmp-rs, mp-tcp-poll, mp-uds-event land. All 12 examples × required schedules × all 7 tier-1 backends green. PRD §11. Placeholder; refine before starting.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 openmp-rs backend (rayon) lands with capabilities.toml.
- [ ] #2 mp-tcp-poll backend lands (nonblocking sockets, busy/yield poll).
- [ ] #3 mp-uds-event backend lands (Unix domain sockets + mio).
- [ ] #4 Remaining examples (8 histogram, 10 wavefront, 12 bitonic sort) land with reference impls.
- [ ] #5 Examples 13 (CNN inference) and 14 (hearing aid) compile and pass tier-1 differential test.
- [ ] #6 Test: 'just e2e --milestone M6' shows full matrix green.
- [ ] #7 Implementation notes record any examples dropped or rescoped for tier-1 feasibility.
- [ ] #8 Implementation notes record honest limitations (perf is not measured; correctness only).
<!-- AC:END -->
