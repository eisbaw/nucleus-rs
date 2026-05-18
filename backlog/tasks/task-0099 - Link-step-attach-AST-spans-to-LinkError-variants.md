---
id: TASK-0099
title: 'Link step: attach AST spans to LinkError variants'
status: To Do
assignee: []
created_date: '2026-05-18 00:42'
labels:
  - compiler
  - link
  - diagnostics
  - M0-followup
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0011's LinkError variants carry only the offending name (kernel, data, loop var). When AST per-node spans land (TASK-0086, TASK-0090), the link step should propagate the originating directive's span onto each LinkError so users get file:line:col on dangling references. Acceptance: each LinkError variant gains an optional Span; messages render with position; tests cover the propagation.
<!-- SECTION:DESCRIPTION:END -->
