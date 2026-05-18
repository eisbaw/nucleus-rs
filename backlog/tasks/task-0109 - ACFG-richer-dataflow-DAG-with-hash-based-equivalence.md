---
id: TASK-0109
title: 'ACFG: richer dataflow DAG with hash-based equivalence'
status: To Do
assignee: []
created_date: '2026-05-18 01:23'
labels:
  - M2
  - compiler
  - ir
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
M1 acfg::DataflowDag is a flat Vec of (data_in, kernel, data_out) edges per firing. The 2013 thesis §4.3.6.1 and the repo's equivalence-by-hashing notes describe a richer per-block DAG that supports common-subexpression elimination at the dataflow level via hash-based equivalence. Promote DataflowDag to a real graph (adjacency list + ports) and implement structural hashing.
<!-- SECTION:DESCRIPTION:END -->
