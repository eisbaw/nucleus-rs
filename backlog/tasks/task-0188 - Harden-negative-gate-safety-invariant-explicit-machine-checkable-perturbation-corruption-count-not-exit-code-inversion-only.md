---
id: TASK-0188
title: >-
  Harden negative-gate safety invariant: explicit machine-checkable
  perturbation/corruption count, not exit-code-inversion only
status: To Do
assignee: []
created_date: '2026-05-19 05:45'
updated_date: '2026-05-19 05:46'
labels:
  - M2
  - backend
  - tech-debt
  - gate-trust
dependencies:
  - TASK-0187
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0187 review gate (mped-architect Finding 1 + Recommendation, non-blocking). The determinism-check-negative AC#2 safety invariant ("the falsifier actually perturbed >=1 tree") is currently encoded SOLELY as the harness exit code, whose meaning is supplied entirely by the inverting shell `if HARNESS; then FAIL; else OK; fi` at justfile:69 — a correct-today but fragile cross-layer coupling: a future refactor of the recipe that drops the inversion would silently re-neuter the falsifier. The committed guard (nucleus/e2e/src/main.rs ~2127-2152, return Ok(0) on zero-perturb-under-gate) is correct, loudly bannered, and regression-tested (zero_perturbation_guard_makes_negative_recipe_fail models the inversion), so this is hardening, not a live defect. Fix: have the harness emit an explicit machine-checkable line (e.g. NUC_NONDET_PERTURBED_CELLS=<n>) on stdout and change justfile:69 to ALSO assert that line, so the safety invariant no longer rests solely on exit-code inversion. The parallel xbackend-check-negative recipe (justfile:85) has the same coupling and must be covered too; TASK-0183 (relocate xbackend wire injection harness-side) will inherit this pattern, so coordinate.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 determinism-check-negative harness emits an explicit machine-checkable perturbed-cell-count signal and justfile:69 asserts it IN ADDITION to the exit-code check (a recipe refactor dropping the inversion fails loud, not silently)
- [ ] #2 xbackend-check-negative (justfile:85) gets the equivalent explicit machine-checkable corrupted-cell-count assertion
- [ ] #3 A test proves the recipe fails loud if the machine-checkable signal says zero perturbations/corruptions even if the exit code alone would invert to OK; determinism-check-negative + xbackend-check-negative still bite 100% (>=5 runs) and bare determinism-check stays byte-identical 30/26/0/4
<!-- AC:END -->
