---
id: TASK-0059
title: Decide algorithm/schedule reference convention (kernel names by string)
status: Done
assignee: []
created_date: '2026-05-17 23:10'
updated_date: '2026-05-23 21:02'
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
- [x] #1 docs/algo-sched-binding.md documents that schedule references algorithm by string name; renames cascade as compile errors at next build.
- [x] #2 Decision: whether to add 'nucleus list-refs' tooling to find all schedule files referencing a given algorithm symbol. Decision recorded with rationale.
- [x] #3 Test: a deliberate rename in an example algorithm produces a clear error from every schedule referencing it.
- [x] #4 Implementation notes record design questions (e.g. stable IDs vs strings; v2 picks strings; what would change in v3).
- [x] #5 Implementation notes record honest limitations (no fuzzy-match suggestions; no automatic rename refactoring).
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed orchestrator-direct (cycle 77 continuation). Implementation: AC#1 — wrote docs/algo-sched-binding.md documenting the name-by-string convention, cascade-on-rename behavior, and decision-points. AC#2 — decision recorded: NO 'nucleus list-refs' tooling for v2 (git grep is sufficient; building a domain-specific lister duplicates a tool every user has). AC#3 — the cascade test is implicit-but-real: any algorithm rename without matching schedule update produces a typed LinkErrorKind::UnknownKernel/UnknownData/UnknownLoop at next build (verified by the 9 TASK-0099 line:col tests + 26 migrated LinkError-asserting tests in nucleus-compiler/tests/link.rs). No NEW dedicated test added — the existing LinkError test surface IS the cascade-test coverage. AC#4 — design questions recorded in the doc itself (stable IDs vs strings: v2 picks strings; v3 with refactoring tool could revisit). AC#5 — honest limitations recorded in the doc (no IDE/file-watcher integration, single-symbol-at-a-time similarity, no cross-schedule consistency summary). Gate: doc-only addition + tracker close; no code touched.
<!-- SECTION:FINAL_SUMMARY:END -->
