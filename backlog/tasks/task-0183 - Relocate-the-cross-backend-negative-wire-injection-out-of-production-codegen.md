---
id: TASK-0183
title: Relocate the cross-backend-negative wire injection out of production codegen
status: To Do
assignee: []
created_date: '2026-05-19 02:55'
updated_date: '2026-05-19 05:13'
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
<!-- SECTION:NOTES:END -->
