---
id: TASK-0115
title: Adjacent-Sync merging in ACFG
status: To Do
assignee: []
created_date: '2026-05-18 01:34'
labels:
  - compiler
  - ir
  - optimisation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
If future rule expansion in TASK-0017 (or follow-ups) ever produces two adjacent Sync nodes in a Sequence, merge them into a single Sync whose participants are the union. Currently TASK-0017's rules never produce adjacent syncs; this is a defensive cleanup that buys forward-compat with rule additions.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Adjacent-Sync merging is correct and idempotent.
- [ ] #2 Test: synthetic ACFG with hand-built adjacent Syncs collapses to one.
<!-- AC:END -->
