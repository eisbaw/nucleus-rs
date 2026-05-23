---
id: TASK-0140
title: 'deadlock pass: emit cycle naming when event DAG is constructible'
status: Done
assignee: []
created_date: '2026-05-18 04:12'
updated_date: '2026-05-23 21:27'
labels:
  - M2
  - compiler
  - validation
  - followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0029 implemented the simulator-replay form of deadlock detection (first stall = deficit place + transition). PRD §8.4's preferred framing is 'cycle in the global event DAG'. When transfer_inject (TASK-0136/0139) is fixed so the net actually has Push→Wait edges, we can layer the DAG cycle-naming on top of the existing pass and enrich DeadlockError with an ordered list of events that form the loop. Today the deficit-place message is correct in both cases, but a true cycle (Push exists but is ordered after the matching Wait, forming a cross-worker rendezvous loop) would be more clearly diagnosed by naming the cycle.

Pre-req: TASK-0136 + TASK-0139 land so the example matrix exercises true cycles, not missing-producer bugs.
<!-- SECTION:DESCRIPTION:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED-no-current-driver (orchestrator-direct, cycle 77 sweep). The pre-req (TASK-0136 + TASK-0139) has landed — net has Push->Wait edges. But the trigger 'when the example matrix exercises true cycles' has not fired: today's example matrix is deadlock-free (none of the 88 e2e cells has ever produced a TASK-0029 deficit-place report on a real schedule). The deficit-place detection has been sufficient. Reopen when a real example produces a true Push-Wait rendezvous cycle that needs the richer DAG-cycle-naming diagnostic — at that point the request is concrete and the implementation is grounded in the actual failure shape, not speculative. Same deferred-no-driver pattern as TASK-0137/0138.
<!-- SECTION:FINAL_SUMMARY:END -->
