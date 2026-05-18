---
id: TASK-0107
title: Scheduler-side validation of Event invariants
status: To Do
assignee: []
created_date: '2026-05-18 01:15'
labels:
  - M1
  - events
  - validation
  - followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0015 deliberately left validation out of the type module. The scheduler must enforce: Push.dst != self_worker, matched (src,dst,data,tile,seq) Push/Wait pairs, non-empty Sync.participants, no overlapping Alloc/Free for the same (data, tile), Free preceded by Alloc on same worker. Reference: PRD §8.2, §8.3, §8.4, TASK-0015 notes.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Scheduler rejects an EventList that violates any documented Event invariant;each invariant has a typed error variant and a negative test
<!-- AC:END -->
