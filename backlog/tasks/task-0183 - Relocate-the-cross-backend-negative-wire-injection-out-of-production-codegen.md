---
id: TASK-0183
title: Relocate the cross-backend-negative wire injection out of production codegen
status: To Do
assignee: []
created_date: '2026-05-19 02:55'
updated_date: '2026-05-19 05:46'
labels:
  - M3
  - backend
  - tech-debt
dependencies:
  - TASK-0178
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
mped-architect-style seam concern, parallel to TASK-0157 (which tracks the same for TASK-0145's NUC_NONDET_TEST). The TASK-0178 NUC_XBACKEND_NEGATIVE perturbation lives inline as maybe_corrupt_wire in mp-tcp-bufsync production lib.rs, called on the wire.rs emission critical path of every shipping build. It is deterministic (fixed source rewrite), value-gated (=='1'), loud-bannered and anchor-guarded (panics if wire_runtime drifts), so it is SAFE — but the seam is not clean: production codegen carries a self-corruption branch. Move it to a #[doc(hidden)] test hook or perform it harness-side (post-process the emitted mp-tcp tree) so production codegen has no test-only branch. Keep behaviour identical; just relocate the seam.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 mp-tcp-bufsync production codegen path contains no test-only corruption branch
- [ ] #2 xbackend-check-negative still bites 100% (>=3 consecutive runs, non-flaky)
- [ ] #3 Loud-banner + value-gate + anchor-drift-panic safety properties preserved
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Forward-carried from TASK-0157 (its determinism-negative analogue, DONE commit e449cac): reuse the SAME clean seam. TASK-0157 deleted the inline branch from pthreads-sync codegen and moved the perturbation harness-side into nucleus/e2e/src/main.rs (maybe_perturb_for_nondet_test, called in check_cell_determinism after both builds, before the diff; perturbs ONE of the two trees so they diverge). For TASK-0183 the harness is the e2e RUN matrix (run_cell), not determinism -- but the principle holds: if the e2e harness is the sole consumer of NUC_XBACKEND_NEGATIVE, post-process the emitted mp-tcp tree harness-side (apply the maybe_corrupt_wire rewrite to wire.rs after nucleus build, before compile/run) so mp-tcp-bufsync production lib.rs carries no self-corruption branch. Keep the exact-"1" gate, loud banner, and the anchor-drift guard (fail LOUD if the wire_runtime anchor is gone -- TASK-0157 used an explicit "codegen layout drifted" Skipped message for the same never-silently-neuter-the-falsifier reason). Verify >=3 consecutive bites per AC#2.

CORRECTION to the forward-carried-from-TASK-0157 seam pattern: do NOT copy the src/main.rs-targeting perturbation verbatim — that exact choice is a partial-silent-neuter bug (TASK-0187): mp-tcp-bufsync emits src/bin/ not src/main.rs, so a src/main.rs-only perturbation silently Skips all mp-tcp cells. Any harness-side relocation here MUST perturb a file every backend emits (e.g. Cargo.toml) or be backend-layout-aware, AND hard-FAIL (not Skip) when zero perturbations happened under the negative env gate. Coordinate with / depend on TASK-0187 so both negative seams use the same provably-biting, never-silently-neuterable harness pattern.

## Forward-carried from TASK-0187 (commit 706065d) — REUSE THIS CORRECTED SEAM

TASK-0187 relocated/hardened the SIBLING NUC_NONDET_TEST perturbation harness-side in nucleus/e2e/src/main.rs. When you move maybe_corrupt_wire harness-side, mirror that exact pattern AND its corrected invariant:

1. **Perturb a layout-agnostic file.** TASK-0187 moved off src/main.rs (pthreads-only) onto Cargo.toml (every backend emits it) because the mp-tcp layout has no main.rs. For xbackend the corruption is mp-tcp-EXCLUSIVE (wire.rs), so target wire.rs in the emitted mp-tcp tree — but ASSERT it exists and treat absence as a hard fail, do NOT let a missing-file Err become a silent Skipped.

2. **THE RECIPE-INVERSION GOTCHA (critical).** justfile xbackend-check-negative, like determinism-check-negative, INVERTS the harness exit code: `if HARNESS; then echo FAIL+exit1; else echo OK`. Harness exit 0 => recipe FAIL; harness exit non-zero => recipe OK. So to make a zero-corruption run a LOUD gate FAIL, the harness must exit CLEAN (0) under the gate — exiting non-zero would let the recipe invert a no-op into a false OK. (TASK-0187 first implemented this backwards and caught it via a live demo; do not repeat.)

3. **Track a corruption-applied count** (analogue of DetCellResult.perturbed threaded through every constructor) and add the zero-corruption guard: under NUC_XBACKEND_NEGATIVE=1, if zero cells were actually corrupted -> loud FATAL + return Ok(0) so the recipe fires its FAIL branch. A falsifier must PROVABLY bite (gate-trust lineage TASK-0145/0157/0163/0167/0178/0187).

4. **Add a unit/integration test** asserting the corruption mutated >=1 tree AND a test modelling the recipe inversion (see TASK-0187 tests in nucleus/e2e/src/main.rs: zero_perturbation_guard_makes_negative_recipe_fail).

5. Env-gate not cfg!/feature (nested cargo --features does not reliably rebuild against the shared target cache — still holds). Keep loud banner + exact-"1" value gate + anchor-drift detection.

Forward-carried from TASK-0187 review gate: TASK-0188 will add an explicit machine-checkable corrupted-cell-count assertion to xbackend-check-negative (justfile:85) so its safety invariant does not rest solely on exit-code inversion. When implementing the harness-side relocation here, coordinate with / depend on TASK-0188 so the xbackend negative seam uses the explicit-signal pattern, not just the inverting recipe.
<!-- SECTION:NOTES:END -->
