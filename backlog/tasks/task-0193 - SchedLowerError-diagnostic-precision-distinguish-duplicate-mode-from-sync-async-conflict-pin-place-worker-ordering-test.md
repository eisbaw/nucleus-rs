---
id: TASK-0193
title: >-
  SchedLowerError diagnostic-precision: distinguish duplicate-mode from
  sync/async conflict + pin place-worker ordering test
status: To Do
assignee: []
created_date: '2026-05-19 15:14'
updated_date: '2026-05-19 15:14'
labels:
  - M0
  - compiler
  - ir
  - diagnostics
dependencies:
  - TASK-0093
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Non-blocking review-gate findings from the TASK-0093/0094/0095 batch (both reviewers GO; both findings explicitly optional/low-priority). (1) ConflictingTransferMode Display says "transfer X is both sync and async" but the same variant also fires on sync,sync / async,async — literally imprecise on the duplicate-mode path. It is DISCLOSED in the variant doc-comment + grammar-sched.md note 7 + commit body (mped-architect adjudicated: recorded, NOT a doc-lie, honest-because-disclosed), so this is precision polish not a correctness/honesty defect. Fix: generalize the Display to be true for both paths (e.g. "transfer X must specify exactly one of sync/async") OR split into a distinct DuplicateTransferMode variant; update grammar-sched.md note 5/7 to document both paths exhaustively (note 5 currently documents only the sync+async case — qa-test-runner P3). (2) Test-hardening: negative_duplicate_place_worker uses declared workers so it does not exercise the duplicate-AND-undeclared ordering path the code comment at lower.rs:234-243 documents (DuplicatePlaceWorker fires before UnknownPlaceWorker). Add a test place k on { ghost, ghost } (ghost undeclared) asserting DuplicatePlaceWorker (not UnknownPlaceWorker) to pin that documented ordering guarantee.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 ConflictingTransferMode diagnostic is literally accurate on BOTH the sync+async and the sync,sync/async,async paths (generalized message or split variant); grammar-sched.md notes 5/7 document both paths
- [ ] #2 A negative test feeds 'place k on { ghost, ghost }' (ghost undeclared) and asserts DuplicatePlaceWorker (not UnknownPlaceWorker), pinning the documented dup-before-undeclared ordering
- [ ] #3 Full gate green (test/e2e 30/26/0/4/0/determinism byte-identical/clippy --all-targets/ci); no SchedLowerError behavioural regression for valid schedules
<!-- AC:END -->
