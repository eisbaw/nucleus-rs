---
id: TASK-0166
title: >-
  Configure GitHub branch protection: ci gate jobs as required status checks
  (AC#2 maintainer action)
status: To Do
assignee: []
created_date: '2026-05-18 22:16'
labels:
  - infra
  - tooling
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Maintainer/settings action that cannot be done from the repo by code. To satisfy TASK-0057 AC#2 'merges blocked on green', the .github/workflows/ci.yml 'gate' matrix jobs must be marked as required status checks on main/master and milestone branches in GitHub repo settings. Blocked until the repo has a GitHub remote. Forward-carried from TASK-0057.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 gate jobs configured as required status checks on protected branches
- [ ] #2 PRs cannot merge while any gate job is red
<!-- AC:END -->
