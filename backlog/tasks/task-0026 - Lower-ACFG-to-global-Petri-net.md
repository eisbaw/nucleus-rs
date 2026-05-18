---
id: TASK-0026
title: Lower ACFG to global Petri net
status: To Do
assignee: []
created_date: '2026-05-17 23:05'
labels:
  - M2
  - compiler
  - ir
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Transform the post-injection ACFG into a global Petri net per PRD §8. Each acfg::operations becomes a transition; each xfer becomes a (push transition + place + wait transition) triple with the place carrying the buffer capacity.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 compiler exposes acfg_to_net(ACFG) -> Net.
- [ ] #2 Pipeline/reuse loop options translate to initial markings on the corresponding places.
- [ ] #3 Buffer=N on a transfer translates to capacity=N on the corresponding place.
- [ ] #4 Test: each example schedule's net is dumped to DOT and snapshot-tested.
- [ ] #5 Implementation notes record design questions (e.g. how to represent control-flow sync as net structure vs as a separate kind of place).
- [ ] #6 Implementation notes record honest limitations (e.g. transfer aggregation may produce a coarser net than necessary).
<!-- AC:END -->
