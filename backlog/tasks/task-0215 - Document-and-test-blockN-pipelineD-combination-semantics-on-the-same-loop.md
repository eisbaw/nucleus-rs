---
id: TASK-0215
title: Document and test block=N + pipeline=D combination semantics on the same loop
status: To Do
assignee: []
created_date: '2026-05-21 14:10'
labels:
  - compiler
  - docs
  - M4
dependencies:
  - TASK-0134
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architecture-review finding (TASK-0134 cycle): when both block= and pipeline= apply to the same loop variable, block_transform tiles the loop into outer/inner; the IterVar id is reused for the inner intra-tile loop (block_transform.rs); pipeline=D therefore applies to the INNER loop. No test exercises this combination; no code-level docstring captures it; users could be surprised by the per-tile (not per-iteration) pipeline-depth semantic.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Either: (a) add a documented + tested semantic (block=N + pipeline=D means D in-flight intra-tile firings per tile; with N>=D the buffer place still gets initial_marking=D); OR (b) reject the combination with a typed error at SchedLowerError or LinkError.
- [ ] #2 Add a synthetic-ACFG unit test that constructs a Repeat-with-block + pipeline= scenario and asserts the chosen semantic; update block_transform.rs module doc with the chosen path.
<!-- AC:END -->
