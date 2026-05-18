---
id: TASK-0168
title: >-
  Standing negative gate: prove the [[required]]-coverage guard still bites
  (wired path)
status: To Do
assignee: []
created_date: '2026-05-18 22:34'
labels:
  - infra
  - tooling
  - quality
  - e2e
dependencies:
  - TASK-0163
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
mped-architect review of TASK-0163 (finding #1). TASK-0163 added required-cell coverage checking with 5 unit tests + a no-gaps-today durable guard, but there is NO standing gate proving the WIRED run() path still exits non-zero on an injected required typo. A future refactor could drop the `if !gaps.is_empty() { return Err }` wiring and all 5 unit tests stay green (they test the pure function, not the harness exit). This is the same class determinism-check-negative solves for determinism. Add an analogous standing negative recipe/test: with an env/flag or a fixture manifest carrying a deliberately-typod required schedule, assert the WIRED nucleus-e2e harness exits non-zero naming the triple; wire it into just ci. Mirror the determinism-check-negative pattern (recipe SUCCEEDS iff harness correctly FAILS).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A standing check (recipe/test in just ci) injects a typod required cell and asserts the wired harness exits non-zero naming the triple
- [ ] #2 It does not leave a broken manifest committed (env/flag or transient fixture, like determinism-check-negative)
- [ ] #3 Removing the run() coverage-gate wiring makes this check fail (proven)
<!-- AC:END -->
