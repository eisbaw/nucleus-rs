---
id: TASK-0015
title: Define Event types (Fire/Alloc/Push/Wait/Sync/Free)
status: To Do
assignee: []
created_date: '2026-05-17 23:04'
labels:
  - M1
  - ir
  - compiler
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Define the EventList contract from PRD §8.3. Six event variants. IterTile, Region, SyncKind, KernelId, DataId, WorkerId, SeqTag as supporting types.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 compiler crate exposes Event enum with 6 variants matching PRD §8.3 exactly.
- [ ] #2 IterTile is Vec<(IterVar, Range<i64>)>.
- [ ] #3 Region is an opaque newtype (an id assigned by the scheduler).
- [ ] #4 SyncKind has exactly one variant: Barrier.
- [ ] #5 Test: Event has Debug, Clone, PartialEq derives; round-trip through serde where useful.
- [ ] #6 Implementation notes record design questions (e.g. should Region carry the memory-region-id name for inspection, or remain pure opaque).
- [ ] #7 Implementation notes record honest limitations (e.g. no end-to-end-latency event; check directive's measurement points are TBD at this stage).
<!-- AC:END -->
