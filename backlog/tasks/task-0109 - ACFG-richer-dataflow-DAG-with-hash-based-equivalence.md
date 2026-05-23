---
id: TASK-0109
title: 'ACFG: richer dataflow DAG with hash-based equivalence'
status: Done
assignee: []
created_date: '2026-05-18 01:23'
updated_date: '2026-05-23 21:27'
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

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED-no-current-driver (orchestrator-direct, cycle 77 sweep). The 'richer per-block DAG + hash-based equivalence for CSE' is a substantive substrate change — it enables common-subexpression elimination at the dataflow level. Today no schedule benefits from CSE: the 11 in-tree examples have no repeated sub-DAGs that would amortise CSE work. Hash-based equivalence is the territory of the separate equivalence-by-hashing/ subproject (TASK-0189/0190/0191 in the To-Do list — labeled as a research track, not on the Nucleus M-line). Reopen if/when a real example surfaces where CSE would change the bit-identical-emitted-code shape AND the equivalence-by-hashing research settles enough to pick an approach. Same deferred-no-driver pattern as TASK-0137/0138.
<!-- SECTION:FINAL_SUMMARY:END -->
