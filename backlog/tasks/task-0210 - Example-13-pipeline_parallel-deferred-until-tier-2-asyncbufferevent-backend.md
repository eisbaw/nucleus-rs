---
id: TASK-0210
title: 'Example 13 pipeline_parallel: deferred until tier-2 async+buffer+event backend'
status: Done
assignee: []
created_date: '2026-05-20 20:13'
updated_date: '2026-05-22 21:08'
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

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 60e tracker hygiene (2026-05-22). The task's premise was OBSOLETED by pthreads-async (cycles 26-27) — a TIER-1 backend that declares supports_async=true + supports_buffer=true + notify=['event']. The task assumed pipeline_parallel needed a tier-2 backend; that assumption proved wrong.

Current state (verified e2e baseline 88/70/0/18):
- 13-cnn-inference/pipeline_parallel × pthreads-async: [[required]] at M4 (cycle 27 / TASK-0229). Bit-identical to reference.bin (sha256 d893337208d7b46923581ecdea8e326e07e8c7e1204a13d867807d6795f7b861).
- 13-cnn-inference/pipeline_parallel × {pthreads-sync, mp-tcp-bufsync}: [[skip]] — capability mismatch (sync/single-buffer/barrier-only) is REAL for those backends. Skip reasons updated cycle 27 to drop the stale TASK-0226 citation.
- 13-cnn-inference/pipeline_parallel × mp-tcp-event: [[skip]] — Stage 3 (TASK-0042.05) deferred. Will move to [[required]] when Stage 3 lands.

Tracker's 'DO NOT add a [[required]] entry on any tier-1 backend' guidance is now OBSOLETE — pthreads-async IS the tier-1 backend with the right capability surface, and the [[required]] entry IS correct. Cycle 27 superseded that guidance with a precise capability-aware comment block in e2e-matrix.toml.

Closing as obsolete-by-implementation. No source changes.
<!-- SECTION:FINAL_SUMMARY:END -->
