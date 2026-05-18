---
id: TASK-0128
title: 'pthreads-sync multi-worker: avoid per-iteration barriers inside loops'
status: To Do
assignee: []
created_date: '2026-05-18 02:51'
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
