---
id: TASK-0165
title: Enable tier-3 Renode CI matrix row (M10)
status: To Do
assignee: []
created_date: '2026-05-18 22:16'
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
