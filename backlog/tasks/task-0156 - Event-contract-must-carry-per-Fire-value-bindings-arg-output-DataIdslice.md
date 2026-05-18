---
id: TASK-0156
title: Event contract must carry per-Fire value bindings (arg/output DataId+slice)
status: To Do
assignee: []
created_date: '2026-05-18 09:41'
labels:
  - M2
  - compiler
  - backend
  - blocker
dependencies:
  - TASK-0150
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Blocks TASK-0124. Event::Fire{kernel,tile} carries no argument/output bindings and no index expressions; acfg_to_events even hardcodes tile=empty. Value-correct codegen (bit-identical e2e) requires knowing, per firing, which (DataId, index-slice) feeds each kernel parameter and which (DataId, slice) it writes — currently only the AlgoIR call/index expressions have this (DataflowEdge::data_in is a bare Vec<DataId>). To let any backend consume ONLY the EventList (TASK-0124 AC#2), extend the Event/ACFG contract to carry per-Fire value bindings, which in turn needs index expressions plumbed through ACFG (TASK-0150). Decide: extend Event::Fire with an arg/out binding payload, or add a sidecar per-firing binding table keyed by (kernel,tile). Must keep determinism + bit-identical e2e.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Event (or a sidecar) carries, per Fire, the ordered input (DataId, slice) bindings and the output (DataId, slice)
- [ ] #2 Index expressions survive ACFG->Event (coordinates with TASK-0150)
- [ ] #3 pthreads-sync can regenerate bit-identical code for examples 01/02/03/05/07 from EventList alone
- [ ] #4 Determinism + bit-identical e2e preserved
<!-- AC:END -->
