---
id: TASK-0139
title: >-
  transfer_inject: emit Push when Wait's matching producer is in an outer scope
  (Repeat-bound consumer case)
status: Done
assignee: []
created_date: '2026-05-18 04:05'
updated_date: '2026-05-18 08:33'
labels:
  - M2
  - compiler
  - bug
  - critical-path
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Currently inject_in_sequence splices Push placeholders only when the matching Wait sits in the same local Sequence as the producer Operation. When the consumer Operation lives inside a Repeat body whose producer is in the outer sequence (e.g. example 02 split: load_input on host outside the for-loop, add on w0 inside), the inner sequence's local_producer_idx is empty so no Push is spliced. The outer scope never sees the Wait either (the Wait is inside the Repeat body), so no Push is emitted at all. The resulting net has wait_seq* transitions with no producer-side push_seq* peers, and the firing simulation deadlocks at the first Wait. Discovered while wiring TASK-0028 boundedness check end-to-end against example 02 split. The boundedness pass surfaces this as BoundednessError::InvalidFiringOrder (a deadlock shape, not an overflow). Acceptance: example 02 split's net contains push_seq* transitions in numbers matching the wait_seq* count, and a derived firing order replays to completion. Acceptance: TASK-0028's e2e_example_02_split_never_overflows_capacity test, once the upstream fix lands, asserts Ok rather than the InvalidFiringOrder fallback.
<!-- SECTION:DESCRIPTION:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Same root cause and fix as TASK-0136 (cross-scope Push/Wait pairing), viewed from the boundedness angle. Resolved by the whole-symbol Pass A/Pass B in transfer_inject. boundedness e2e (e2e_example_02_split_never_overflows_capacity) already tolerated Ok(()); it now takes the ideal Ok path (1 push / 1 wait per symbol, capacity-1 buffer never overflows) instead of the InvalidFiringOrder deadlock fallback. Verified by the same test run as TASK-0136.
<!-- SECTION:FINAL_SUMMARY:END -->
