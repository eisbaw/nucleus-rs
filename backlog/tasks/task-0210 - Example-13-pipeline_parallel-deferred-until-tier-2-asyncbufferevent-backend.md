---
id: TASK-0210
title: 'Example 13 pipeline_parallel: deferred until tier-2 async+buffer+event backend'
status: To Do
assignee: []
created_date: '2026-05-20 20:13'
labels:
  - M5
  - examples
  - capability-gap
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Example 13's pipeline_parallel.sched.nuc declares:
  transfer input  : async, buffer=3, notify=event;
  transfer feat1  : async, buffer=3, notify=event;
  transfer feat2  : async, buffer=3, notify=event;

Both tier-1 backends advertise (capabilities.toml):
  supports_async  = false
  supports_buffer = false
  max_buffer      = 1
  notify          = ("barrier", "blocking")

The capability-compat check correctly rejects pipeline_parallel for both
pthreads-sync and mp-tcp-bufsync with messages of the form:
  transfer X requests `async` but the backend does not support async transfers
  ...
This is the fail-loud capability machinery working as designed (PRD §13
"Capability mismatch ... must report these as early compile errors with
named missing capabilities").

Consequence: pipeline_parallel cannot enter the tier-1 cross-backend
differential matrix. It is a tier-2 / M5+ feature workload — needs a
backend that exposes async streaming + buffer>1 + event notify.

This task is the placeholder so the gap is tracked, NOT silently
papered over. Specifically:

- DO NOT add a [[required]] entry for example 13 / pipeline_parallel in
  e2e-matrix.toml on any tier-1 backend.
- DO NOT add a [[skip]] for pipeline_parallel on tier-1 backends either
  (the e2e-matrix.toml comments say SKIP is for cells the harness
  should report as informational; a capability-mismatched cell does not
  belong there). The harness already plans non-required cells as
  informational on best-effort, and the capability check fails them
  loudly — which is the correct surface.
- When a tier-2 async/event backend lands (post-M5), revisit and
  promote pipeline_parallel to a required cell on THAT backend
  (matching the schedule's actual capability needs).

Dependencies:
- TASK-0117 (distributed placement) is a likely precondition for the
  worker-to-worker pipeline edges.
- A future tier-2 / M5+ backend task that introduces async + buffer + event.

Not blocking TASK-0053: example 13's deliverable can land naive (and
batch_parallel on pthreads-sync once TASK-0209 closes), with
pipeline_parallel honestly deferred.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Determination: pipeline_parallel.sched.nuc is NOT promoted to required on any tier-1 backend; the schedule file remains shipped.
- [ ] #2 When an async + buffer + event-capable backend lands, this task is closed by adding pipeline_parallel as a required cell on that backend with a bit-identical reference.bin match.
<!-- AC:END -->
