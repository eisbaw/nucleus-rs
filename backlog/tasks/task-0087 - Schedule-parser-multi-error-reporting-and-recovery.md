---
id: TASK-0087
title: 'Schedule parser: multi-error reporting and recovery'
status: To Do
assignee: []
created_date: '2026-05-18 00:13'
labels:
  - M0
  - compiler
  - language
  - followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0008 self-report follow-up. The schedule parser currently bails on the first syntax error. For a usable DX, we want to report multiple errors per pass and recover at directive boundaries (the semicolon between SchedItems is a natural sync point). Chumsky's recovery primitives support this; not done in TASK-0008 to keep that task scoped. Same follow-up applies to the algorithm parser.
<!-- SECTION:DESCRIPTION:END -->
