---
id: TASK-0453
title: 'Rigour + shortcomings: implementation/thesis ping-pong'
status: To Do
assignee: []
created_date: '2026-06-06 22:51'
labels:
  - rigour
  - thesis
  - epic
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
EPIC. Drive more rigour by turning the thesis's honestly-documented shortcomings (paper/chapters/10-discussion.tex limitations + threats-to-validity, paper/chapters/11-future-work.tex, and defence-prep weaknesses W1-W5 in TASK-0452.08) into PLANNED, dependency-linked IMPLEMENT+THESIS-UPDATE task pairs, then ping-pong: implement a fix under full phase3 discipline, then revise the thesis section documenting that shortcoming to reflect the new capability HONESTLY (residual kept as documented limitation). FUNDAMENTAL trade-offs (expressiveness-vs-deterministic-firing etc.) stay honest limitations and are NOT planned away (see the Fundamental-limitations register child). Rule: strengthen ACTUAL rigour, never relabel; every claim codebase-verified; no regressions (just ci GREEN, e2e baseline held, thesis PDF green).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Each ADDRESSABLE shortcoming has a dependency-linked IMPLEMENT->THESIS-UPDATE pair filed
- [ ] #2 FUNDAMENTAL limitations registered as honest-not-planned
- [ ] #3 Ping-pong cycles land both sides (code + thesis) per cycle with gates green
<!-- AC:END -->
