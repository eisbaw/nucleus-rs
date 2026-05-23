---
id: TASK-0165
title: Enable tier-3 Renode CI matrix row (M10)
status: Done
assignee: []
created_date: '2026-05-18 22:16'
updated_date: '2026-05-23 20:58'
labels:
  - infra
  - tooling
  - M10
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Re-enable the disabled tier-3 matrix include row in .github/workflows/ci.yml when M10 (first Renode shim) lands. Per PRD §10.3 compile-mandatory + run-in-Renode-where-supported (examples 1,5,9). Referenced by a code comment in ci.yml (TASK-0057). Renode is a heavy emulator — likely self-hosted runner; see TASK-0057 AC#6/#7 notes.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 ci.yml tier-3 include row enabled for milestone M10
- [ ] #2 Row runs no_std compile + Renode runtime validation for examples 1,5,9, not the tier-1 just ci gate
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED-to-M10 (orchestrator-direct, cycle 77 sweep). Labeled M10; same pattern as TASK-0164 but for the Renode tier-3 matrix row instead of MPI tier-2. M10 (first Renode shim) not in progress yet. Reopen when M10 lands and the tier-3 Renode backend has a CI-runnable shape.
<!-- SECTION:FINAL_SUMMARY:END -->
