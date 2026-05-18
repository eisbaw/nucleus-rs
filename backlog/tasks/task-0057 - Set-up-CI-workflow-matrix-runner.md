---
id: TASK-0057
title: Set up CI workflow (matrix runner)
status: To Do
assignee: []
created_date: '2026-05-17 23:10'
labels:
  - infra
  - tooling
  - M0
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Set up CI (likely GitHub Actions or self-hosted GitLab CI) that runs 'just check', 'just clippy', 'just test', and 'just e2e' inside nix develop. Matrix runs per milestone.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 .github/workflows/ (or .gitlab-ci.yml) runs jobs: check, clippy, test, e2e — all inside 'nix develop' or a Nix-shell wrapper.
- [ ] #2 CI exits non-zero on any failure; merges blocked on green.
- [ ] #3 Matrix runner is parameterised by milestone label; PRs to milestone branches run the relevant tier.
- [ ] #4 Test: a deliberate clippy warning fails CI.
- [ ] #5 Test: an e2e cell failure shows up clearly in the workflow output.
- [ ] #6 Implementation notes record design questions (e.g. self-hosted runner vs GitHub-hosted; cost; cache strategy for Nix and Cargo).
- [ ] #7 Implementation notes record honest limitations (e.g. no tier-3 Renode CI until M10; tier-2 MPI CI lands at M7).
<!-- AC:END -->
