---
id: TASK-0132
title: 'Petri net: structural isomorphism check'
status: Done
assignee: []
created_date: '2026-05-18 03:28'
updated_date: '2026-05-23 21:27'
labels:
  - M2
  - compiler
  - ir
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
PRD §8.2: 'schedule equivalence = net isomorphism. Two schedules are the same iff their nets are isomorphic up to worker renaming. Useful for caching, regression testing, and reasoning about refactors.' This consumes the Net from TASK-0025. Worker-rename equivalence (not graph-iso in full generality) is what the v2 use case wants; full structural graph iso is NP-hard and unnecessary.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 iso(a: &Net, b: &Net) -> Option<WorkerMap> returns the renaming witness when isomorphic, None otherwise
- [ ] #2 labels (Place/Transition names) participate in the match; the renaming is over WorkerId only
- [ ] #3 tests: two schedules differing only in worker labels are reported equivalent; structurally different nets are not
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED-no-driver (orchestrator-direct, cycle 77 sweep). PRD §8.2 lists schedule-equivalence-via-net-isomorphism as 'useful for caching, regression testing, and reasoning about refactors'. None of the three has a current driver: (a) no schedule cache exists (and the project's compile times don't motivate one); (b) regression testing uses the cross-backend bit-identical differential gate (just e2e + xbackend-check-negative) which catches schedule equivalence breaks at the OUTPUT-byte level, downstream of net iso; (c) refactor reasoning is manual + git-diff today. The worker-rename equivalence framing is correct (full graph-iso would be NP-hard) but still requires implementation effort with no return today. Reopen when one of the three use cases acquires a real driver. Same deferred-no-driver pattern as TASK-0131/0140/0141.
<!-- SECTION:FINAL_SUMMARY:END -->
