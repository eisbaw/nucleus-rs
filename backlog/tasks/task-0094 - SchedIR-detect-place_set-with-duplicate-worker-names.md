---
id: TASK-0094
title: 'SchedIR: detect place_set with duplicate worker names'
status: To Do
assignee: []
created_date: '2026-05-18 00:33'
labels:
  - M0
  - compiler
  - ir
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
place k on { w0, w0 } is currently accepted by lower_sched (TASK-0010). Either reject as a hard error or fold to a unique set. PRD sec 6.3.2 doesn't speak to the duplicate case; pick a rule and enforce it.
<!-- SECTION:DESCRIPTION:END -->
