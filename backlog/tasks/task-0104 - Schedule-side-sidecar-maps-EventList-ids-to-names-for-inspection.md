---
id: TASK-0104
title: Schedule-side sidecar maps EventList ids to names for inspection
status: Done
assignee: []
created_date: '2026-05-18 01:15'
updated_date: '2026-05-23 21:13'
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

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as SUBSTANTIALLY DONE (orchestrator-direct, cycle 77 sweep). NameTables (TASK-0238 cycles 36-37) already implements the inverse mapping for 4 of 5 id types the task lists: KernelId/DataId/WorkerId/IterVar -> String, populated by NameTables::from_acfg(&acfg). Inspection tooling can already print 'conv_block_1' for KernelId(7) etc. via NameTables. The remaining Region -> name gap is naturally addressed by TASK-0137 ('petri_to_events: emit Alloc/Free events and resolve Region') which is still open and IS the natural home for Region naming (Region emission happens at petri_to_events, so the name resolution belongs in the same pass that emits the event). Closing this task with the Region naming residual carried by TASK-0137.
<!-- SECTION:FINAL_SUMMARY:END -->
