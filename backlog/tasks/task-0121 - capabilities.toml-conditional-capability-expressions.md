---
id: TASK-0121
title: capabilities.toml conditional capability expressions
status: To Do
assignee: []
created_date: '2026-05-18 01:58'
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
