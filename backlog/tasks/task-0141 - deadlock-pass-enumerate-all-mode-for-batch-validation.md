---
id: TASK-0141
title: 'deadlock pass: enumerate-all mode for batch validation'
status: To Do
assignee: []
created_date: '2026-05-18 04:12'
labels:
  - M2
  - compiler
  - validation
  - followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0029 reports the first stall and stops. For a batch-validation pass over many examples, an 'enumerate all stalls reachable from the same initial marking' mode is useful — it shows the user every wait_seq* that would deadlock, not just the earliest. Implementation sketch: after a stall, mark the offending Wait as 'ignore' and continue replay; record every Wait whose precondition place stays empty through the rest of the order. Only worth doing if user feedback asks for it.
<!-- SECTION:DESCRIPTION:END -->
