---
id: TASK-0104
title: Schedule-side sidecar maps EventList ids to names for inspection
status: To Do
assignee: []
created_date: '2026-05-18 01:15'
labels:
  - M1
  - events
  - inspection
  - followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0015 left EventList ids as opaque u64. Inspection tooling that wants to print 'conv_block_1' for KernelId(7) or 'shared_sram' for Region(2) needs a sidecar map produced by the schedule pass. Produce KernelId/DataId/WorkerId/IterVar/Region -> name maps alongside the per-worker EventList. Reference: PRD §8.3, TASK-0015 notes.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Schedule pass emits sidecar id-to-name maps per event-id family;CLI inspection commands consume the maps to render human-readable event listings;maps are serde-compatible with the event wire format
<!-- AC:END -->
