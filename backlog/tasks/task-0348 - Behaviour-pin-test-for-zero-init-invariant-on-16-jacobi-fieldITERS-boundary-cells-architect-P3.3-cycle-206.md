---
id: TASK-0348
title: >-
  Behaviour-pin test for zero-init invariant on 16-jacobi field[ITERS] +
  boundary cells (architect P3.3 cycle 206)
status: To Do
assignee: []
created_date: '2026-05-27 14:24'
labels:
  - examples
  - test-pin
  - defensive
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
=== Filed as TASK-0341.02 cycle-206 architect P3.3 follow-up ===

16-jacobi's correctness rests on an unstated invariant: data symbols not explicitly assigned by any Dataflow stmt are pre-initialised to 0 by codegen. Specifically:

- field[ITERS] is read by the kernel at t==0 via the `(t+ITERS)%(ITERS+1)` index (the modular-wrap trick), then ignored when t==0 (kernel returns seed_yx).
- field boundary cells (y in {0, H-1} or x in {0, W-1}) are never written and stay 0 (Dirichlet BC).
- result is also implicitly 0-initialised before the result-extract loop (though the loop covers the full grid, so this isn't load-bearing for result).

The invariant lives in shared helpers (e.g. nucleus-backend-common/src/lib.rs ~281/457 zero-init helper for output Vec allocations).

## Why a pin

Same precedent as TASK-0303 / TASK-0304 narrative-pin tests for the M5 stencil examples. A behaviour-pin test guards against silent regression: if a future cycle changes the zero-init contract (e.g. uses `Vec::with_capacity` + push instead of `vec![0; ...]`), 16-jacobi (and 11-game-of-life) would emit garbage in the boundary cells and the e2e cmp would catch it — but the test would also catch the regression at the unit-test layer, with a more precise diagnostic.

## Acceptance criteria

1. **Test file**: a behaviour-pin test in nucleus-backend-common/tests/ (or a similar shared location) named `task0341_02_zero_init_invariant.rs` that exercises a build of 16-jacobi/naive/pthreads-sync, inspects the generated main.rs string, and asserts the zero-init pattern at the field allocation site.
2. **Sibling coverage**: same pin for 11-game-of-life (which has the same invariant).
3. **Architect review-GO**: pin lands without flakiness; test runs in unit-test profile.

## Honest scope LIMITS

- This is a defensive pin, not a feature. Low priority — file only if the zero-init contract is shaping or changing in a future cycle.
<!-- SECTION:DESCRIPTION:END -->
