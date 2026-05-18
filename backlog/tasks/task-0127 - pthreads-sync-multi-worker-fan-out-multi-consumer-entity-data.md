---
id: TASK-0127
title: 'pthreads-sync multi-worker: fan-out (multi-consumer-entity) data'
status: To Do
assignee: []
created_date: '2026-05-18 02:51'
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
