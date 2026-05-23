---
id: TASK-0251
title: >-
  Promote NUC_*_NEGATIVE per-test env-mutex to a project-wide one when a 3rd
  injection seam lands
status: To Do
assignee: []
created_date: '2026-05-23 15:28'
labels:
  - infra
  - tooling
  - test-hygiene
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

Cycle 71 review (mped-architect MINOR-3) flagged a latent test-env hazard: each of the three negative-gate seams (`NUC_NONDET_TEST`, `NUC_XBACKEND_NEGATIVE`, `NUC_REQUIRED_COVERAGE_NEGATIVE`) currently uses a per-test mutex (e.g. `req_cov_neg_env_lock()`) to serialize its own unit-tests. `std::env::set_var` is process-global, so any future test in `nucleus/e2e` that indirectly invokes `run_inner` or one of the `maybe_*` injection functions would not be protected — the per-seam mutex only fences against tests of the same seam.

Today this is **latent, not active**: repo-wide grep confirms each env var is only read in (a) its 4 sibling tests + (b) the single production function. So none of the three seams' tests can collide with each other (different vars, different mutexes, no shared callee). The hazard becomes real only if a fourth env-flag injection seam lands, or if a non-`req_cov_*` test starts calling `run_inner`.

## Class of bug, not a regression

This is shared discipline across all three negative-gate seams. The cycle 71 implementation does NOT make it worse; it just adds a third instance of the same per-seam mutex pattern.

## Recommended fix when activated

Promote the per-seam `OnceLock<Mutex<()>>` to a single project-wide `ENV_TEST_MUTEX` in `nucleus/e2e/src/main.rs` (or a `test_common`-equivalent shared helper). Any test that touches process-global env state takes the same lock. The three existing per-seam locks become aliases / forwarders.

## Acceptance criteria

- [ ] #1 Trigger condition reached (4th env-flag injection seam or non-`req_cov_*` test invokes a `maybe_*` injector).
- [ ] #2 Single shared env-test mutex; all three (or more) negative-gate test families take the same lock.
- [ ] #3 Documented in the module-level header comment so future contributors don't reinvent per-seam locks.

## Dependencies

None today. Filed as cycle-71 forward-carry from TASK-0168.
<!-- SECTION:DESCRIPTION:END -->
