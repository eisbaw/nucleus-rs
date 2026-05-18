---
id: TASK-0167
title: Genuine per-milestone CI parameterisation (wire --milestone end to end)
status: To Do
assignee: []
created_date: '2026-05-18 22:23'
labels:
  - infra
  - tooling
  - M1
dependencies:
  - TASK-0057
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
mped-architect review of TASK-0057 found the CI milestone matrix was cosmetic (7 identical jobs; the e2e harness --milestone flag is accepted-but-ignored per nucleus/e2e/src/main.rs ~1526; nuc-nucleus/e2e-matrix.toml has no milestone dimension). The decorative matrix was removed and AC#3 of TASK-0057 honestly unchecked. This task is the REAL work AC#3 wanted: (1) make the e2e harness honour --milestone (subset the required cells by milestone); (2) add a milestone key to each cell in e2e-matrix.toml; (3) reinstate a CI matrix keyed on milestone that actually runs a different required set per milestone; (4) PRs to a milestone branch run that milestone tier. Until then PRD §11 milestone-gating is not real.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 e2e harness honours --milestone: required-set is subset by milestone, verified by a test
- [ ] #2 e2e-matrix.toml carries a per-cell milestone key
- [ ] #3 CI matrix runs a genuinely different required set per milestone (not identical jobs)
- [ ] #4 A PR to a milestone branch runs that milestone tier (documented + matrix wired)
<!-- AC:END -->
