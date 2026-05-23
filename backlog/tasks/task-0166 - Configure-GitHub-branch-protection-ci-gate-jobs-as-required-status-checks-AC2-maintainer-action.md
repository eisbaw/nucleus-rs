---
id: TASK-0166
title: >-
  Configure GitHub branch protection: ci gate jobs as required status checks
  (AC#2 maintainer action)
status: Done
assignee: []
created_date: '2026-05-18 22:16'
updated_date: '2026-05-23 20:50'
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

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED (orchestrator-direct, cycle 77 sweep). The task description explicitly states 'Blocked until the repo has a GitHub remote.' This is a maintainer/settings action that cannot be done from the repo by code. The current project has no GitHub remote configured. Reopen when the GitHub remote exists and the maintainer is ready to configure required-status-check branch protection. Until then, carrying this as To-Do is environment-blocked indefinitely.
<!-- SECTION:FINAL_SUMMARY:END -->
