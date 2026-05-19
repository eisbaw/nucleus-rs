---
id: TASK-0178
title: Prove the cross-backend differential gate bites (M3 negative arm)
status: To Do
assignee: []
created_date: '2026-05-19 01:05'
labels:
  - M3
  - validation
  - quality
dependencies:
  - TASK-0036
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0041 AC#5: the cross-backend e2e differential must be PROVEN to bite, analogous to determinism-check-negative (TASK-0145) and the required-coverage guard (TASK-0163). Deliberately perturb one mp-tcp-bufsync cell (e.g. flip a sign / off-by-one in the shared renderer reachable only via the mp-tcp path, or a transport encode bug) and assert just e2e / CI FAILS that cell with required-fail>0 — then revert; the durable guard is a test/recipe, not a committed broken backend (mirror the determinism-check-negative pattern: recipe SUCCEEDS iff the harness correctly FAILS). Without this, "differential green" is only the positive arm — a false-negative in the cross-backend falsifier would go unnoticed.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A standing negative check perturbs an mp-tcp cell and asserts the e2e/CI differential FAILS it (required-fail>0), then is non-destructive (env/flag or transient, like determinism-check-negative)
- [ ] #2 Wired into just ci
- [ ] #3 Reverting the perturbation returns the gate to green (proven)
<!-- AC:END -->
