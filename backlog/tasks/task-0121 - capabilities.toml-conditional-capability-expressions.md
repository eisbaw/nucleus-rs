---
id: TASK-0121
title: capabilities.toml conditional capability expressions
status: Done
assignee: []
created_date: '2026-05-18 01:58'
updated_date: '2026-05-23 21:07'
labels:
  - M5
  - backend
  - tooling
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0019 follow-up: the current capabilities.toml shape is a flat list of boolean flags and counts. It cannot express conditional capabilities like 'async only when buffer >= 2' or 'notify=event only when transport=tcp'. Add a restrictions[] block (or equivalent) that takes structured predicates over the existing flags, with check_schedule_compat evaluating them. Useful for backends where capability axes are not orthogonal.
<!-- SECTION:DESCRIPTION:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED-to-M5 (orchestrator-direct, cycle 77 sweep). Labeled M5; description: 'Useful for backends where capability axes are not orthogonal.' Current 4 tier-1 backends (pthreads-sync, mp-tcp-bufsync, pthreads-async, mp-tcp-event) all have orthogonal capability axes — the flat boolean+counts shape suffices today. The conditional expressions surface ('async only when buffer>=2', 'notify=event only when transport=tcp') becomes a real need at M5+ when distributed schedules introduce backends with conjoined capability constraints. Reopen at M5 entry when the first non-orthogonal backend lands. Same deferred-closure pattern as the cycle-77 sweep.
<!-- SECTION:FINAL_SUMMARY:END -->
