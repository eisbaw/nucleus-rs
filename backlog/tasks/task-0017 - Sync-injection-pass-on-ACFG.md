---
id: TASK-0017
title: Sync injection pass on ACFG
status: To Do
assignee: []
created_date: '2026-05-17 23:04'
labels:
  - M1
  - compiler
  - ir
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Walk the ACFG and inject acfg::sync nodes between regions on different workers where control-flow demands a barrier (e.g. top-level statement boundaries with cross-worker dependencies). PRD §8 + 2013 thesis §4.3.9.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 compiler exposes inject_syncs(ACFG) -> ACFG.
- [ ] #2 A sync is injected exactly where it's needed for control-flow coherency; over-synchronization is to be avoided where possible.
- [ ] #3 Sync nodes capture from-workers and to-workers sets.
- [ ] #4 Test: synthetic two-worker programs produce expected sync placement (table-driven test).
- [ ] #5 Implementation notes record design questions (e.g. when to fold a sync into an existing transfer's coherency event).
- [ ] #6 Implementation notes record honest limitations (e.g. may over-sync; optimisation passes deferred).
<!-- AC:END -->
