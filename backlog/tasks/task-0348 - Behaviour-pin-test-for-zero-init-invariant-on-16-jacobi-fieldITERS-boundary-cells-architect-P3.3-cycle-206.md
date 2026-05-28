---
id: TASK-0348
title: >-
  Behaviour-pin test for zero-init invariant on 16-jacobi field[ITERS] +
  boundary cells (architect P3.3 cycle 206)
status: Done
assignee:
  - '@mark'
created_date: '2026-05-27 14:24'
updated_date: '2026-05-28 03:14'
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 226 implementation plan (orchestrator-direct)

Empirically verified BEFORE writing any assertion (Layer-3 discipline, per the cycle-225 18th-firing lesson in feedback-orchestrator-narrative-also-wrong — verify equivalence/behaviour claims against the actual artifact, not the narrative):

Built both examples via release binary + inspected emitted main.rs:
- 16-jacobi/naive/pthreads-sync: `let mut field = vec![0; 320];` (line 9) + `let mut result = vec![0; 64];` (line 10). 320 = (ITERS+1)*H*W = 5*8*8; 64 = H*W = 8*8. Confirmed data decls: field: i32[ITERS+1][H][W], result: i32[H][W].
- 11-game-of-life/naive/pthreads-sync: `let mut grid = vec![0; 288];` (line 9) + `let mut result = vec![0; 32];` (line 10). 288 = (ITERS+1)*N = 9*32; 32 = N. Confirmed data decls: grid: i32[ITERS+1][N], result: i32[N].

### Test design

Location: nucleus/driver/tests/task0348_zero_init_invariant.rs (driver crate can see the compiled nucleus binary via env!(CARGO_BIN_EXE_nucleus); mirrors cli_reuse_strict.rs subprocess pattern — runs in unit-test profile under `just test`, no cargo-run recursion).

Per example: run `nucleus build --backend pthreads-sync --out <tmp>` on the naive schedule, read <tmp>/src/main.rs, assert:
1. Primary (load-bearing invariant): main.rs contains `let mut field = vec![0;` (resp. grid) — the zero-fill that makes (a) field[ITERS] read at t==0 via the (t+ITERS)%(ITERS+1) wrap return 0, and (b) Dirichlet boundary cells stay 0.
2. Precision pin: exact line `let mut field = vec![0; 320];` (resp. `grid ... 288`).
3. result zero-init: `let mut result = vec![0;`.

Message distinguishes 'zero-init contract broke (real regression)' from 'dimensions changed (update expected size)'.

### AC mapping
AC#1 16-jacobi pin: this test.
AC#2 game-of-life sibling: same test, second case.
AC#3 architect review-GO + no flakiness + unit-test profile.

### Gate
build, clippy, test, test-release, e2e (280/246/0/34/0 must hold — test-only, no codegen change).

## Cycle 226 + 226b closure

Behaviour-pin for the zero-init allocation strategy on 16-jacobi field + 11-game-of-life grid landed. Commits:
- d477b50 tests: cycle 226 — task0348_zero_init_invariant.rs (2 tests; values empirically verified vs artifact; pin proven to bite)
- 5309b2e tests: cycle 226b — review-gate fold-back (architect P2 docstring overclaim correction)

### Review gate
- qa-test-runner: GO. New test 3x deterministic (no flakiness — AC#3). Dev 1037/0/3, release 1036/0/3, e2e 280/246/0/34/0, only test+tracker changed.
- mped-architect: GO. P2 (docstring overclaimed vec![0;N] AS the invariant — folded back in 226b). P3 (result-zero-init pins are pure documentation — honestly disclosed, kept). Confirmed sizes correct (320=5*8*8, 288=9*32), kernel claims accurate, NO silent sibling (the modular-wrap shape exists in ONLY these 2 examples).

### ACs (final)
AC#1 16-jacobi field pin: TICKED.
AC#2 11-game-of-life grid sibling pin: TICKED.
AC#3 architect review-GO + no flakiness + unit profile: TICKED (architect GO; qa 3x deterministic; runs under just test via CARGO_BIN_EXE subprocess, cli_reuse_strict.rs precedent).

### Honest scope (architect P2 — important nuance)
This is an ALLOCATION-STRATEGY pin, NOT a full semantic-invariant pin. vec![0;N] is necessary-but-not-sufficient. The full semantic invariant (field[ITERS] never WRITTEN before the t==0 read + interior-loop bounds leave boundary cells unwritten) is guarded END-TO-END by the e2e differential vs reference.bin. A codegen that zero-filled AND populated the top slice would keep this unit test green while breaking the invariant. The 226b docstring fix states this explicitly.

### Considered-but-not-filed avenue
A deeper static pin (parse emitted main.rs, assert no write to field[ITERS] precedes the modular-wrap read) would close the proxy-vs-semantic gap at the unit layer. NOT filed: the e2e differential already guards the semantic invariant, and a structural never-written-before-read assertion against generated Rust is high-effort/low-marginal-value (would re-implement dataflow analysis the compiler already does). Mentioned here so a future cycle that wants unit-layer semantic coverage has the pointer.

### Gotchas / forward-carried lessons
1. clippy::doc_lazy_continuation (feedback-clippy-doc-lazy-continuation-recurring) bit on first draft: a paragraph following an unclosed '- ' bullet list with NO blank '//!' separator is read as a lazy list continuation. Fix: blank '//!' line between a bullet list and the following paragraph + single-line bullets. ALWAYS re-run just clippy independently after writing a multi-line //! or /// block with list items.
2. Driver-crate tests reach the compiled nucleus binary via env!(CARGO_BIN_EXE_nucleus) — the clean unit-profile pattern for 'build an example + inspect emitted artifact' tests (no cargo-run recursion, no cargo-build of the generated project). Mirror cli_reuse_strict.rs.
3. Behaviour-pin doc discipline: distinguish 'pins the invariant' from 'pins the allocation strategy the invariant relies on'. The architect P2 caught the conflation. When a unit pin asserts a PROXY for a semantic property, say so — and name the green-but-broken corner the pin can't catch + which layer (e2e) guards the real property.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Cycles 226 + 226b landed a behaviour-pin (nucleus/driver/tests/task0348_zero_init_invariant.rs, 2 tests) for the zero-fill allocation strategy on 16-jacobi field (vec![0;320]) + 11-game-of-life grid (vec![0;288]) + their result arrays. Built each example's naive/pthreads-sync schedule via CARGO_BIN_EXE subprocess, inspected emitted main.rs, asserted the vec![0; zero-fill prefix (load-bearing) + exact size (precision). Pin proven to bite (wrong-size -> FAILED, reverted). Review gate: qa GO (3x no-flakiness, 1037/0/3, e2e 280/246/0/34/0), architect GO. Architect P2 folded back in 226b: docstring corrected to label this an ALLOCATION-STRATEGY pin (necessary-but-not-sufficient proxy), with the full semantic invariant (never-written-before-read + unwritten-boundary) guarded end-to-end by the e2e differential. All 3 ACs ticked.
<!-- SECTION:FINAL_SUMMARY:END -->
