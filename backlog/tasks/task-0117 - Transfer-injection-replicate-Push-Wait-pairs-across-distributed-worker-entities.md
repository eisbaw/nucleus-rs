---
id: TASK-0117
title: >-
  Transfer-injection: replicate Push/Wait pairs across distributed worker
  entities
status: To Do
assignee: []
created_date: '2026-05-18 01:44'
labels:
  - M1
  - compiler
  - ir
  - follow-up
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
transfer_inject collapses 'place k on {w0,w1,w2,w3}' into a single canonical WorkerId for the src/dst on XferPlaceholder. A future partition pass (TASK-0016+ alignment) needs to fan out one Push/Wait pair per worker in the set, partitioned by loop.partition=... policy. Spec: when crossing from a singleton {host} into a distributed entity {w0..w3}, the schedule's transfer directive plus the loop.partition= should produce N pairs, each carrying its slice of the IterTile.
<!-- SECTION:DESCRIPTION:END -->
