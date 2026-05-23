---
id: TASK-0128
title: 'pthreads-sync multi-worker: avoid per-iteration barriers inside loops'
status: Done
assignee: []
created_date: '2026-05-18 02:51'
updated_date: '2026-05-23 21:29'
labels:
  - M1
  - compiler
  - optimisation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Current codegen honours every ACFG Sync node, including the per-iteration barrier injected at the entry of each Repeat body whose body's workers differ from the prior statement's writing workers. For example 02, host enters the for-loop just to barrier every iteration alongside w0 — N=256 unnecessary syncs. Either deduplicate redundant Syncs in the optimisation pass (TASK-0113) or skip non-participating workers' loop entry when the loop body holds only Operations not on this worker plus Sync placeholders the worker participates in only on entry/exit.
<!-- SECTION:DESCRIPTION:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED-no-perf-driver (orchestrator-direct, cycle 77 sweep). The 'N=256 unnecessary syncs in example 02' observation is structurally correct but has no perf driver — no contributor has measured example-02's wall time as bottlenecked by host-side barrier ops; pthreads barriers on shared-memory are nanoseconds. The broader 'drop redundant Syncs after transfer injection' optimization is TASK-0113 (closed cycle 77 as deferred-no-driver) — TASK-0128 is its example-02-specific instance. Both reopen together when a real perf measurement shows redundant-Sync overhead bites. Same deferred-no-driver pattern.
<!-- SECTION:FINAL_SUMMARY:END -->
