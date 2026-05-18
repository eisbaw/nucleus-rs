---
id: TASK-0059
title: Decide algorithm/schedule reference convention (kernel names by string)
status: To Do
assignee: []
created_date: '2026-05-17 23:10'
labels:
  - compiler
  - docs
  - M0
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD §12 notes that schedule references kernels and loop variables by string name; renaming the algorithm silently invalidates schedules. Document the convention explicitly and decide whether to add any tooling support (e.g. 'nucleus check --algo X.algo.nuc' that lists all schedule files referencing it).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 docs/algo-sched-binding.md documents that schedule references algorithm by string name; renames cascade as compile errors at next build.
- [ ] #2 Decision: whether to add 'nucleus list-refs' tooling to find all schedule files referencing a given algorithm symbol. Decision recorded with rationale.
- [ ] #3 Test: a deliberate rename in an example algorithm produces a clear error from every schedule referencing it.
- [ ] #4 Implementation notes record design questions (e.g. stable IDs vs strings; v2 picks strings; what would change in v3).
- [ ] #5 Implementation notes record honest limitations (no fuzzy-match suggestions; no automatic rename refactoring).
<!-- AC:END -->
