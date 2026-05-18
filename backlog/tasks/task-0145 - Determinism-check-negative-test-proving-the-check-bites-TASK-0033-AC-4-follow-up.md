---
id: TASK-0145
title: >-
  Determinism check: negative test proving the check bites (TASK-0033 AC #4
  follow-up)
status: To Do
assignee: []
created_date: '2026-05-18 05:06'
labels:
  - M2
  - validation
  - tooling
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0033 AC #4 asked for a deliberate-nondeterminism injection that proves the determinism check fails loud. The TASK-0033 implementation provides the positive-arm test (deterministic codegen produces PASS) but defers the negative arm. A clean way to do it: a feature-gated test in the pthreads-sync backend that iterates a HashMap when emitting (e.g. slot ID enumeration) instead of the current BTreeMap. With the gate off the e2e matrix stays green; with the gate on the determinism check must FAIL with an offending file path. Acceptance: the gated test compiles, exercises the gate, and the determinism harness emits a non-zero exit with a useful pointer at the offending file. Without this, AC #4 is technically not met — only AC #1/#2/#3/#5/#6 are exercised by the current run.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Feature-gated codegen path introduces HashMap iteration in the pthreads-sync backend
- [ ] #2 Determinism check exits non-zero when the gate is on, pointing at the offending file path
- [ ] #3 Default (gate off) keeps the existing PASS matrix green
<!-- AC:END -->
