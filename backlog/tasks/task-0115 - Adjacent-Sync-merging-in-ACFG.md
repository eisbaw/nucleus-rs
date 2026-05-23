---
id: TASK-0115
title: Adjacent-Sync merging in ACFG
status: Done
assignee: []
created_date: '2026-05-18 01:34'
updated_date: '2026-05-23 21:13'
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

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED-defensive (orchestrator-direct, cycle 77 sweep). Description: 'Currently TASK-0017's rules never produce adjacent syncs; this is a defensive cleanup that buys forward-compat with rule additions.' TASK-0017 sync-injection rules have been stable through 7 keystone cycles + the TASK-0218 barrier-skipping optimization (cycle ~37). No rule addition has produced adjacent syncs and none is planned. Reopen IF a future sync_inject rule addition produces adjacent syncs (which would surface as an analysis-net oddity caught by boundedness or deadlock). Same defensive-deferred pattern as the cycle-77 sweep.
<!-- SECTION:FINAL_SUMMARY:END -->
