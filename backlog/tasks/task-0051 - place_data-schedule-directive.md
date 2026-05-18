---
id: TASK-0051
title: place_data schedule directive
status: To Do
assignee: []
created_date: '2026-05-17 23:09'
labels:
  - language
  - compiler
  - M9
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Bind algorithm data symbols to declared memory regions. Required for tier-3 where TCM vs shared SRAM matters. PRD §6.3.1.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Schedule directive 'place_data D in MEMORY_REGION' parses, lowers, and is validated against the worker classes accessing D.
- [ ] #2 A data symbol with no place_data uses the default region per the placement workers' class.
- [ ] #3 Conflict (data accessed from a class that can't reach the chosen region) is a compile error.
- [ ] #4 Alloc events carry the resolved Region tag.
- [ ] #5 Test: example 14 has place_data directives that lower correctly.
- [ ] #6 Test: a deliberately mismatched place_data produces a clear error.
- [ ] #7 Implementation notes record design questions (e.g. per_worker memory regions vs shared; how aliasing rules interact with place_data).
- [ ] #8 Implementation notes record honest limitations (no automatic placement; user must state).
<!-- AC:END -->
