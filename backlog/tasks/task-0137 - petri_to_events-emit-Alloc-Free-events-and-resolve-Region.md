---
id: TASK-0137
title: 'petri_to_events: emit Alloc/Free events and resolve Region'
status: To Do
assignee: []
created_date: '2026-05-18 03:50'
updated_date: '2026-05-23 14:27'
labels:
  - compiler
  - M3
  - ir
  - follow-up
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0027 elided Event::Alloc / Event::Free for M2: the pthreads-sync backend uses on-stack allocation and has no need. PRD §8.3 specifies these events and the Region tag; the schedule sublanguage (PRD §6.3.1) has `place_data D in MEMORY_REGION`, but that surface is not yet threaded through the link/ACFG passes either.

This follow-up does:
1. Plumb `place_data` directives through SchedIR -> LinkedIR -> ACFG (and ultimately into the per-data Region assignment).
2. Augment `petri_to_events` to scan each worker's EventList for first-use/last-use of each data symbol and inject Alloc/Free at those positions.

Open design Q: do Alloc/Free live in the ACFG (so multiple passes can see them) or do they get synthesised purely in the projection step? The PRD's §5 pipeline diagram suggests the former; the projection-only form is simpler for M2.5.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Each data symbol has an Alloc emitted on first use per worker, and a Free emitted after last use.
- [ ] #2 Region is derived from schedule directives (place_data D in MEMORY_REGION); when absent, the backend's default region is used.
- [ ] #3 Tests cover the Alloc/Free positions in a multi-worker schedule with explicit place_data.
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0107 cycle 67: this task is the upstream producer for the currently-LATENT invariants (4) OverlappingAlloc and (5) FreeWithoutAlloc in `nucleus/compiler/src/event_validate.rs`. petri_to_events.rs:113 documents 'Event::Alloc / Event::Free are NOT emitted' today, so the validator paths for (4) and (5) are only exercised by synthetic tests. When this task lands and emits real Alloc/Free events, the debug_assert at petri_to_events.rs:238 will exercise (4)/(5) on real input for the first time — that is the moment a real Alloc/Free bug would surface. Also: the validator's current Loop-recursion handling flattens nested events without modeling multi-iteration replay; for a Loop body that allocates without freeing inside the body, multi-iteration backend replay would alias on the second iteration but the validator only walks the body once. Re-examine that semantic when Alloc/Free codegen lands (see event_validate.rs:480-484 comment).
<!-- SECTION:NOTES:END -->
