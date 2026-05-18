---
id: TASK-0056
title: 'Schedule completeness checks: unplaced kernels, dangling references'
status: To Do
assignee: []
created_date: '2026-05-17 23:10'
labels:
  - compiler
  - language
  - M0
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD §12 risks: a schedule that omits placement for any kernel is a hard error; placing a kernel that doesn't exist in the algorithm is also a hard error. Loop variables and data symbols similarly. Implement the comprehensive check.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Linking phase reports ALL missing placements and ALL dangling references in one pass, not just the first.
- [ ] #2 Each error names the offending symbol and a one-line hint (e.g. 'kernel X declared at algo.nuc:42 has no place directive in sched.nuc').
- [ ] #3 Test: a schedule missing two placements reports both, not just one.
- [ ] #4 Test: a schedule referencing a non-existent kernel produces UnknownKernel error pointing at the schedule line.
- [ ] #5 Implementation notes record design questions (e.g. should we suggest typo-fix via fuzzy matching; v2 says no).
- [ ] #6 Implementation notes record honest limitations (kernel names are stringly typed; no stable IDs across versions).
<!-- AC:END -->
