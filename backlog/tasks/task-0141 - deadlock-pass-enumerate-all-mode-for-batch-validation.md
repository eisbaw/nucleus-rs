---
id: TASK-0141
title: 'deadlock pass: enumerate-all mode for batch validation'
status: Done
assignee: []
created_date: '2026-05-18 04:12'
updated_date: '2026-05-23 21:08'
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

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED (orchestrator-direct, cycle 77 sweep). Description: 'Only worth doing if user feedback asks for it.' No user feedback. The first-stall report from TASK-0029 has been sufficient for current example matrix; no batch-validation use case has surfaced where seeing every reachable wait-deadlock matters more than fixing the first one. Reopen if a user (or a future M5+ many-schedule batch-validate flow) actually asks for enumerate-all-stalls. Same deferred-closure pattern as the cycle-77 sweep.
<!-- SECTION:FINAL_SUMMARY:END -->
