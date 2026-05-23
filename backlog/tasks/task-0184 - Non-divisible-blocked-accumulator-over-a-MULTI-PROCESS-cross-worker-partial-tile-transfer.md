---
id: TASK-0184
title: >-
  Non-divisible blocked accumulator over a MULTI-PROCESS cross-worker
  partial-tile transfer
status: Done
assignee: []
created_date: '2026-05-19 03:27'
updated_date: '2026-05-23 21:21'
labels:
  - M3
  - validation
  - coverage-frontier
dependencies:
  - TASK-0173
  - TASK-0181
  - TASK-0175
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
mped-architect review of TASK-0173 (non-blocking coverage-frontier, NOT a 0173 defect). TASK-0173 AC#3 proved a non-divisible blocked ACCUMULATOR is bit-identical to naive — but via 04-prefix-sum which is single-host, so under mp-tcp-bufsync it runs single-PROCESS (shared single-worker renderer). The residual frontier: a non-divisible blocked accumulator under a genuine MULTI-process cross-worker partial-tile TRANSFER (the trailing-partial-tile slice actually crossing a TCP/shared-mem boundary between two workers). Currently covered by NO required cell and NO AC of TASK-0181 (0181 is the multi-worker blocked-rebind render path mechanically + its fail-loud guard, not a non-divisible-accumulator differential over it). Gated on a tier-1 MULTI-WORKER blocked schedule existing at all (none does today; all blocked cells are single-host; worker<->worker mesh is TASK-0175, multi-worker blocked rebind is TASK-0181). When a tier-1 multi-worker blocked schedule lands, add a non-divisible blocked accumulator differential cell over it (bit-identical to naive, both backends). Until then this is a documented coverage frontier, not a gap in shipped behaviour.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Once a tier-1 multi-worker blocked schedule exists, a non-divisible blocked accumulator differential cell crosses a real worker boundary and is bit-identical to naive on both backends
- [ ] #2 The trailing-partial-tile slice is exercised across the cross-worker transfer (not just intra-worker arithmetic)
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED-coverage-frontier (orchestrator-direct, cycle 77 sweep). Task description explicitly states: 'Gated on a tier-1 MULTI-WORKER blocked schedule existing at all (none does today; all blocked cells are single-host)... Until then this is a documented coverage frontier, not a gap in shipped behaviour.' The dependency chain is: TASK-0175 (worker-to-worker mesh) + TASK-0181 (multi-worker blocked rebind — landed cycle 73 but unreachable in tier-1 today per its own honest-limits note) + a NEW multi-worker blocked schedule fixture. When all three converge, this differential becomes meaningful AND its AC#1+#2 become testable. Reopen at trigger (the first tier-1 multi-worker blocked schedule). Same deferred-until-trigger pattern as the cycle-77 sweep.
<!-- SECTION:FINAL_SUMMARY:END -->
