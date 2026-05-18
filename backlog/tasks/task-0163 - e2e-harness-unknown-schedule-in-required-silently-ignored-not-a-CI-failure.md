---
id: TASK-0163
title: >-
  e2e harness: unknown schedule in [[required]] silently ignored, not a CI
  failure
status: To Do
assignee: []
created_date: '2026-05-18 22:15'
updated_date: '2026-05-18 22:23'
labels:
  - infra
  - tooling
  - e2e
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Found during TASK-0057 CI work. Adding a [[required]] entry to nuc-nucleus/e2e-matrix.toml whose schedule does not match a discovered *.sched.nuc file does NOT fail the e2e run — the harness only walks discovered schedule files, so a typo'd or stale required cell vanishes silently instead of FAILing. This is a CI blind spot: a required cell can be lost without anyone noticing. Harness should error if a [[required]] (example,schedule,backend) triple is not discovered/executed. Forward-carried from TASK-0057.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Harness exits non-zero if any [[required]] matrix entry is not matched to an executed cell
- [ ] #2 Error message names the unmatched required triple
- [ ] #3 e2e-matrix.toml typo of a required schedule is caught by just e2e
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Raised to HIGH (mped-architect review of TASK-0057, P2): the entire project + CI gate trusts the e2e harness `required-fail: 0` line. If a [[required]] cell with a typo/stale schedule silently vanishes instead of failing, a required cell can be deleted by a one-char typo with GREEN CI — a false-negative in the falsifier itself, the exact class determinism-check-negative exists to prevent but unguarded for the required matrix. Foundational, not deferrable. Should be treated as gating trust in TASK-0057 / TASK-0167. Forward-carried into TASK-0167 (genuine milestone matrix must not reintroduce this).
<!-- SECTION:NOTES:END -->
