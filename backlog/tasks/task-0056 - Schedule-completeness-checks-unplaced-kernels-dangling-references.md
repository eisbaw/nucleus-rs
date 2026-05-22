---
id: TASK-0056
title: 'Schedule completeness checks: unplaced kernels, dangling references'
status: Done
assignee: []
created_date: '2026-05-17 23:10'
updated_date: '2026-05-22 21:05'
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
- [x] #1 Linking phase reports ALL missing placements and ALL dangling references in one pass, not just the first.
- [x] #2 Each error names the offending symbol and a one-line hint (e.g. 'kernel X declared at algo.nuc:42 has no place directive in sched.nuc').
- [x] #3 Test: a schedule missing two placements reports both, not just one.
- [x] #4 Test: a schedule referencing a non-existent kernel produces UnknownKernel error pointing at the schedule line.
- [x] #5 Implementation notes record design questions (e.g. should we suggest typo-fix via fuzzy matching; v2 says no).
- [x] #6 Implementation notes record honest limitations (kernel names are stringly typed; no stable IDs across versions).
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycle 60b tracker hygiene (2026-05-22). All 6 ACs structurally met by pre-session work. Link layer fires UnplacedKernel + UnknownKernel + UnknownData + UnknownLoop + UnknownTransferData errors, all multi-error (TASK-0092 accumulator). 22+ tests in compiler/tests/link.rs pin each variant + multi-violation reporting. The fuzzy-matching did-you-mean is a separate decision (TASK-0096) per cycle-history. v2 says no fuzzy suggestions — honest limitation per AC#5/6.
<!-- SECTION:FINAL_SUMMARY:END -->
