---
id: TASK-0039
title: 'Example 4: prefix sum (scan) — algorithm + naive + blocked + reference'
status: To Do
assignee: []
created_date: '2026-05-17 23:07'
labels:
  - M3
  - examples
  - validation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Two-pass scan algorithm. Stresses ordering between two passes that share a worker. At M3, used to test naive + blocked.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 examples/04-prefix-sum/ has algo, schedules (naive, blocked), kernels.rs, reference/, input.bin, reference.bin.
- [ ] #2 Algorithm expressed as two sequential loops (upsweep, downsweep) or equivalent; integer-typed to stay deterministic.
- [ ] #3 Test: passes M3 differential matrix on both pthreads-sync and mp-tcp-bufsync.
- [ ] #4 Implementation notes record design questions (e.g. how to encode the two-pass pattern without procedure abstraction in Nuc).
- [ ] #5 Implementation notes record honest limitations (parallel scan tree is not used here; this is sequential-style for simplicity, since v2 doesn't have prefix-scan as a built-in).
<!-- AC:END -->
