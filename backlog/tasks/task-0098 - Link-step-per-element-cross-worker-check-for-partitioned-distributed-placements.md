---
id: TASK-0098
title: >-
  Link step: per-element cross-worker check for partitioned distributed
  placements
status: To Do
assignee: []
created_date: '2026-05-18 00:42'
labels:
  - compiler
  - link
  - M5-followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0011 currently treats a distributed placement ('place K on {w0..w3}') as a single worker entity for the cross-worker transfer-existence check. This is correct for whole-tensor data movement but misses per-element halo cases: e.g. blur3 placed on {w0..w3} with partition=rows reading img_in[y-1] across the row boundary needs a transfer for the halo even though producer and consumer are 'the same set'. This is dependent on TASK-0016+ partition= lowering. Acceptance: once partition= is resolved, the link step (or a sibling pass) sees the partitioned iteration space and detects per-partition cross-worker reads against the placement set. Filed now so the limitation is tracked.
<!-- SECTION:DESCRIPTION:END -->
