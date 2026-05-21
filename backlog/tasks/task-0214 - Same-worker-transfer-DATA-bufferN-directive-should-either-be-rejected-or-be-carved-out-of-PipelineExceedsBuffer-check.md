---
id: TASK-0214
title: >-
  Same-worker transfer DATA : buffer=N directive should either be rejected or be
  carved out of PipelineExceedsBuffer check
status: To Do
assignee: []
created_date: '2026-05-21 14:10'
labels:
  - compiler
  - link
  - M4
  - latent
dependencies:
  - TASK-0134
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architecture-review finding (TASK-0134 cycle): the link-step PipelineExceedsBuffer check fires whenever a data symbol with a transfer directive has D > N and both producer/consumer kernels are inside the pipelined loop. The IR (transfer_inject) does NOT emit an Xfer for same-worker producer/consumer (src==dst at transfer_inject.rs:1717). So a pathological schedule with transfer X : buffer=1 on a same-worker symbol + pipeline=3 would emit PipelineExceedsBuffer despite no actual constraint existing in the lowered IR. Latent inconsistency. Currently the link.rs:669-685 doc-comment acknowledges this and points at this task; the in-tree examples don't hit it.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Decide one path: (a) reject 'transfer X : buffer=N' when src==dst is structurally inevitable from the schedule, with a SchedLowerError naming X and the placement that makes it same-worker; OR (b) gate check_pipeline_buffer_constraints on the kernel placements (skip when producer/consumer share a worker).
- [ ] #2 Test: positive — same-worker producer/consumer with redundant transfer directive AND pipeline=2 + buffer=1 must either fail at SchedLower (path a) or link cleanly (path b). Document the choice in link.rs:669-685 with the new ground truth (replacing the current 'Caveat TASK-0214' note).
- [ ] #3 Forward-carry into TASK-0042.01: if path (a) is chosen, pthreads-async codegen never sees same-worker transfer directives; if path (b), it must skip same-worker transfers in its ring-buffer setup.
<!-- AC:END -->
