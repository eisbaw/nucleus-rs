---
id: TASK-0274
title: >-
  Driver-CLI integration test for reuse-inference strict-error wrapping
  (TASK-0271 follow-up)
status: To Do
assignee: []
created_date: '2026-05-24 09:16'
labels:
  - M5
  - test-gap
  - driver
  - reuse
  - forward-carried-from-TASK-0271
dependencies:
  - TASK-0271
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

Forward-carried from TASK-0271 cycle-88 architect review (P3 finding).

TASK-0271 promoted `nucleus/driver/src/main.rs:413` from `apply_reuse_inference_advisory` to `apply_reuse_inference`, wrapping the typed `ReuseInferenceError` via `.map_err(|e| format!("reuse-inference error: {e}"))?`. The negative test `tests/sidecar_reuse.rs::task0271_strict_rejects_non_affine_reuse_body` pins the strict function's behaviour at the pass level — but does NOT pin the driver's outer wrapping (the `reuse-inference error: ` prefix + the surfacing as a CLI error).

## Gap

A future refactor that:
- swaps `format!("reuse-inference error: {e}")` for `e.to_string()` (drops prefix), OR
- changes the wrapping prefix text, OR
- removes the `?` and silently logs instead

…would NOT be caught by any existing test. The pass-level pin doesn't cover the driver call site's contract; the e2e matrix only exercises SUCCESS fixtures.

## Acceptance

1. New integration test (probably under `nucleus/driver/tests/`) that invokes the `nucleus build` CLI on a fixture with a non-affine reuse-tagged loop AND asserts:
   - exit code is non-zero,
   - stderr (or stdout, whichever the driver writes errors to) contains the substring "reuse-inference error:" AND the variant-specific Display text (e.g. "strided access not supported").
2. The fixture file lives alongside the existing test fixtures (NOT inline in the test body — fixture file makes the negative-shape grep-able).
3. The fixture is small and synthetic — e.g. a single-stmt algorithm with `grid[V*2]` strided access inside a `loop V : reuse;` body.
4. Test runs in CI; just e2e + just determinism-check stay GREEN.

## Honest scope

LOW priority. The current pass-level pin already proves the strict variant bites. This task adds belt-and-braces coverage for the driver wrapping. File now so a future refactor doesn't silently change the user-visible error surface.

## Dependencies

- Forward-carried from: TASK-0271 (cycle-88 architect P3 review item).
- Related: TASK-0273 (multi-worker marker coverage gap; both are coverage-gap follow-ups from cycle-87/88).
<!-- SECTION:DESCRIPTION:END -->
