---
id: TASK-0369
title: >-
  Tier-1 fault-path differential cell: on_violation=count/log check-loop trips
  latency_max — zero tier-1 differential coverage today
status: To Do
assignee: []
created_date: '2026-05-30 11:08'
labels:
  - e2e
  - runtime-check
  - robustness
  - M?
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-213 strategic-analysis finding (R3, robustness). VERIFIED: on_violation (count/log/panic) appears ONLY in embedded schedules (01-elementwise-add/embedded_check*, 14-hearing-aid/embedded_multimcu*), all req=0 in the tier-1 e2e matrix (validated solely by separate Renode recipes). So the entire runtime-assertion + fault-reporting surface is NEVER cross-backend differential-tested on tier-1 — the bit-identity invariant has zero coverage for the fault path. The inject_check_frames machinery already exists for tier-1 (TASK-0052). Add a tier-1 check-loop schedule that deliberately trips latency_max with on_violation=count (and/or log), producing a deterministic fault-report artifact, and promote it across the tier-1 backends.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A tier-1 (non-embedded) schedule exists with a check loop V : latency_max=T, on_violation=count (and/or log) where T is set so the violation deterministically FIRES
- [ ] #2 The fault-report output (the count/log artifact) is bit-identical across the tier-1 backends where the schedule is capability-compatible, and the cell is promoted [[required]] on those backends
- [ ] #3 Honest scoping: if on_violation=count/log output is inherently non-deterministic across backends (e.g. timing-derived), document precisely what IS pinned (e.g. the violation-count, not the latency value) and pin only that
<!-- AC:END -->
