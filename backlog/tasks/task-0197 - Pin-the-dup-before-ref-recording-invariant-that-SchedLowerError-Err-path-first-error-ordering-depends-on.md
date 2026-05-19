---
id: TASK-0197
title: >-
  Pin the dup-before-ref-recording invariant that SchedLowerError Err-path
  first-error ordering depends on
status: To Do
assignee: []
created_date: '2026-05-19 17:16'
updated_date: '2026-05-19 17:16'
labels:
  - M0
  - compiler
  - diagnostics
  - tech-debt
  - test
dependencies:
  - TASK-0196
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Non-blocking latent-fragility surfaced by the TASK-0196 mped-architect review (minor obs #2; both reviewers GO, explicitly no follow-up REQUIRED — this tracks it per backlog-as-working-memory so it cannot silently rot). TASK-0196 option (b) relocated UnknownWorkerClass/UnknownAccessibleByName into pass-1 AST-walk side-tables (worker_class_refs/accessible_by_refs) and proved first-error ordering byte-equivalent to the old name-sorted BTreeMap iteration via a stable sort. That equivalence holds ONLY because worker/region name uniqueness holds, which in turn holds ONLY because dup-detection (DuplicateWorker/DuplicateMemoryRegion) early-returns BEFORE any ref tuple is recorded. This ordering invariant is comment-documented but IMPLICIT: a future refactor moving dup-detection after ref-recording would silently change which error a user sees first on a multi-fault schedule — and the determinism gate would NOT catch it (it only covers valid input; this is an Err-path property). Add an explicit guard so the invariant cannot silently break.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 An explicit mechanism pins the invariant: either a debug_assert/structural guard that ref-recording happens only after the dup guards, OR a regression test feeding a multi-fault schedule (a duplicate worker AND an unknown worker-class) asserting the SAME error fires first as the documented behaviour, so a refactor reordering dup-detection vs ref-recording fails loudly
- [ ] #2 Full gate green (just test/e2e 30/26/0/4/0/determinism byte-identical/clippy --all-targets/ci); no behaviour change
<!-- AC:END -->
