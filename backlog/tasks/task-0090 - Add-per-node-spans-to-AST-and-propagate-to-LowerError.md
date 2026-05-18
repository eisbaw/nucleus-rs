---
id: TASK-0090
title: Add per-node spans to AST and propagate to LowerError
status: To Do
assignee: []
created_date: '2026-05-18 00:25'
labels:
  - M0
  - compiler
  - diagnostics
  - follow-up
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Both the parser AST and the lowering pass currently lack span tracking on individual nodes. Once AST nodes carry (line, column), LowerError variants should gain position fields. Surface stays source-compatible; just enriches diagnostics. Filed as follow-up from TASK-0007 and TASK-0009.
<!-- SECTION:DESCRIPTION:END -->
