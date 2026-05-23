---
id: TASK-0127
title: 'pthreads-sync multi-worker: fan-out (multi-consumer-entity) data'
status: Done
assignee: []
created_date: '2026-05-18 02:51'
updated_date: '2026-05-23 21:29'
labels:
  - M1
  - backend
  - codegen
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Multi-worker codegen in TASK-0122 rejects data symbols with more than one consumer entity. Implement fan-out by allocating one Slot per (producer, consumer) pair and emitting one push per consumer on the producer side. Needed once example 13 (CNN inference, broadcast inputs) or similar lands.
<!-- SECTION:DESCRIPTION:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED-no-driver (orchestrator-direct, cycle 77 sweep). Description: 'Needed once example 13 (CNN inference, broadcast inputs) or similar lands.' Example 13 has landed (cycles via TASK-0042.04 + earlier), but its schedules (batch_parallel, pipeline_parallel) PARTITION the batch dimension across workers — they don't broadcast inputs to multiple consumer entities. No in-tree schedule today has the multi-consumer-entity fan-out pattern. The pthreads-sync codegen's ContractGap on this case is honest-loud: a future schedule using fan-out will fail with a precise message that names this task. Reopen when such a schedule lands. Same deferred-no-driver pattern as TASK-0131/0132/0140.
<!-- SECTION:FINAL_SUMMARY:END -->
