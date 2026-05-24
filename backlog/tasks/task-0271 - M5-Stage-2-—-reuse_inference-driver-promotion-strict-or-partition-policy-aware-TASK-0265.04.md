---
id: TASK-0271
title: >-
  M5 Stage 2 — reuse_inference driver promotion strict or partition-policy-aware
  (TASK-0265.04)
status: Done
assignee:
  - '@mped-architect-impl'
created_date: '2026-05-24 08:33'
updated_date: '2026-05-24 09:10'
labels:
  - M5
  - driver
  - reuse
  - stage-2
  - forward-carried-from-TASK-0265
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Forward-carried from TASK-0265 cycle 87 — review item 1 of 5.

The current Stage 1 driver call site (driver/src/main.rs:410) consumes apply_reuse_inference_advisory and swallows every typed ReuseInferenceError via nuc_trace. That was correct in Stage 1 because no consumer read reuse_widths. Tier 1 Stage 2 wiring (commit 7d03606) wires a walker-side reader (marker emit at body entry) so the cost-of-silent-swallow rises: a non-affine index now silently has NO entry in reuse_widths, so the consumer renders no marker AND emits no buffer, yet the user wrote loop V : reuse. Future Tier 2/3 (real codegen TASK-0269/0270) raises the cost further — a silently-skipped slot becomes silently-correct-but-unoptimised code, surprising the user.

## Two policies to choose between
A. Strict: switch to apply_reuse_inference and treat any typed error as fatal. Pure. Simple. Rejects every reuse-tagged loop whose body is not affine.
B. Partition-policy-aware: keep advisory but escalate to fatal when the err's iv is tagged with a partition= directive on the same loop OR a Stage 2 consumer is about to read its slot. Same shape halo Stage 2 may need; consider lifting a shared passes::common::iv_diag_policy helper if both pass diagnostics surface in the same driver pass.

## Coordination with TASK-0260 halo
Halo has the same choice today (its driver also uses advisory). Both should be solved together — the user-visible diagnostic is the same shape, and the policy lives in the driver, not the pass. Possible TASK-0265.04+0260-sibling lift.

## Review-item context (cycle-82 architect)
Promote driver from lenient apply_reuse_inference_advisory to strict. Once codegen reads reuse_widths, a silently-swallowed typed error becomes wrong output. Pick partition-policy-aware fatality (same pattern Stage 2 of TASK-0260 needs to apply for halo).

## AC
1. Decide between (A) and (B), document the decision in the driver comment + pass module docs.
2. Update driver/src/main.rs:410 call site.
3. New tests pin the new fatal/advisory boundary.
4. Existing examples (which today are affine-only) still pass; if any silently-non-affine body exists, surface it and decide cell-by-cell.
5. just e2e + just determinism-check stay GREEN.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
POLICY CHOICE: (A) STRICT.

Rationale:
1. Every shipped reuse-tagged schedule today (only 05-stencil/reuse, since 05-stencil/distributed is [[skip]] on every backend per TASK-0267/0268) has an affine 3x3 stencil body equivalent to the in-pass positive_3point_stencil test. Strict promotion therefore does NOT change the e2e matrix; baseline 92/77/0/15/0 should hold byte-identical.
2. Tier 1 (TASK-0265 cycle 87) ALREADY wired a marker consumer at Event::Loop emit. So the (B) predicate 'is this slot consumed?' is trivially TRUE for every slot today — (B) degenerates into (A).
3. (B)'s alternative predicate 'iv carries partition=' is too narrow: a non-partitioned reuse loop with a marker consumer (the exact 05-stencil/reuse shape) would still emit no marker for a non-affine body, recreating the silent-failure mode.
4. MPED: fail-fast, fail-loud, no silent drops. Driver returns typed Err -> human-readable diagnostic via existing Display impl for ReuseInferenceError.

Implementation steps:
1. driver/src/main.rs:391-420: replace apply_reuse_inference_advisory call with apply_reuse_inference (strict). Convert returned Err to formatted 'reuse-inference error: {e}' driver-level String error using the same pattern as partition_rows/partition_blocks2d at lines 347/360.
2. Rewrite the long driver comment block to reflect the new policy. Reference TASK-0271 as the promotion. Note halo (driver line 385) is still advisory, owned by TASK-0263.
3. Remove unused import apply_reuse_inference_advisory; add apply_reuse_inference.
4. reuse_inference.rs module docs: rewrite the 'Strict vs advisory entry points' subsection (lines 72-88) to reflect that the driver now uses STRICT. Keep apply_reuse_inference_advisory exported (used by in-pass tests).
5. apply_reuse_inference_advisory doc-comment 'Stage 1 driver policy' subsection: rewrite to past-tense + cross-link the strict promotion.
6. New tests in tests/sidecar_reuse.rs (NOT in passes/reuse_inference.rs's #[cfg(test)] mod, which already has these; the driver-facing pin belongs in the integration test crate):
   a. strict_rejects_non_affine_reuse_body — synthetic non-affine (e.g. grid[V*2]) under loop V : reuse; → apply_reuse_inference returns Err(StridedAccessNotSupported{coefficient:2,..}).
   b. strict_accepts_shipped_05_stencil_reuse — re-load 05-stencil/reuse.sched.nuc through the strict apply, assert is_ok() (regression pin against future drift).

7. Verification:
   - just e2e MUST stay 92/77/0/15/0.
   - just determinism-check MUST stay GREEN.
   - cargo test --workspace MUST stay 0-failed; new tests included.
   - cargo clippy --workspace --all-targets -- -D warnings MUST stay clean.

8. Forward-carry follow-ups (file as new tasks if found):
   - Diagnostic quality of ReuseInferenceError::Display already audited (cycle 82, sees variant test) — no follow-up.
   - passes::common::iv_diag_policy lift: defer to TASK-0263 closure (currently 1 caller, would be premature).

9. Append cycle notes to TASK-0271; forward-carry the 'strict pattern' bullet to TASK-0263's notes.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 88 (2026-05-24) — LANDED Done

### WHAT LANDED
- Driver call at nucleus/driver/src/main.rs:413-414 promoted from apply_reuse_inference_advisory (lenient, nuc_trace! swallow) to apply_reuse_inference (strict, '?'-propagate). Commit 0a74bea.
- nucleus/nucleus-compiler/src/passes/reuse_inference.rs module docs (lines 72-93) rewritten: strict-vs-advisory section now names the driver as strict consumer, advisory entry point doc-comment rewritten as test-only.
- Two new pins in nucleus/nucleus-compiler/tests/sidecar_reuse.rs:
  * task0271_strict_rejects_non_affine_reuse_body (synthetic grid[V*2] under loop V : reuse; → typed StridedAccessNotSupported{coefficient:2,..}).
  * task0271_strict_accepts_shipped_05_stencil_reuse_schedule (regression pin re-loading 05-stencil/reuse through the strict path + asserting reuse_widths populated).

### POLICY CHOICE
**(A) STRICT.** Tier 1 marker consumer means every slot is consumed today, so (B)'s 'is this consumed?' predicate is trivially TRUE for every slot — (B) degenerates into (A). (B)'s narrower 'iv carries partition=' rule would still silently drop non-affine reuse on non-partitioned loops (the 05-stencil/reuse shape), recreating the failure mode. MPED: fail-fast, fail-loud.

### ACTUAL GATE NUMBERS (post-commit, all GREEN)
- just e2e: 92 / 77 / 0 / 15 / 0 (byte-identical to baseline)
- just determinism-check: 92 / 77 / 0 / 15 GREEN
- just determinism-check-negative: NUC_NONDET_PERTURBED_CELLS=77 (falsifier fires; gate not trivially passing)
- just test: 0-failed across 30+ test buckets (sidecar_reuse 8 passed including 2 new)
- cargo clippy --workspace --all-targets -- -D warnings: clean

### PER-AC STATUS
- AC#1 (decide A vs B + document): MET. (A) strict; rationale documented in driver comment block (main.rs:391-420) + reuse_inference.rs module docs + commit body.
- AC#2 (update driver/src/main.rs:410 call site): MET. Lines 391-420 (the line-numbering shifted as the comment block expanded).
- AC#3 (new tests pin the new fatal boundary): MET. task0271_strict_rejects_non_affine_reuse_body pins the fatal arm; task0271_strict_accepts_shipped_05_stencil_reuse_schedule pins the accepting arm.
- AC#4 (existing examples still pass): MET. e2e 92/77/0/15/0 byte-identical; no shipped schedule has a non-affine reuse body.
- AC#5 (just e2e + just determinism-check stay GREEN): MET. Both verified above.

### FOLLOW-UPS FILED
None — Display impls on ReuseInferenceError were already audited at cycle 82 (variant-coverage tests exist for 6/8 variants, defensive 2/8 added cycle 87), and the passes::common::iv_diag_policy lift would be premature with a single strict-driver caller. When TASK-0263 promotes halo's driver call, the duplication will be real and the lift can be filed then.

### HONEST LIMITS
- The negative test fixture builds an inconsistent body shape only at the IR level (uses Mul as the index expr); no parse-level coverage of 'a user typed loop V : reuse; over a strided body' end-to-end. This is OK because the affine detector lives ONE level above the parser (it walks IrExpr post-link), but a future grammar change to reject Mul-in-index at parse time would make this synthetic shape unreachable. Re-evaluate at that point.
- The strict promotion does NOT change the sidecar serde shape (cycle-87 contract pin holds), so no NameSidecar contract-version bump needed.
- apply_reuse_inference_advisory is STILL exported (used by reuse_inference.rs in-pass advisory_collects_all_errors_strict_short_circuits and the determinism pin). Its doc-comment now warns the driver no longer consumes it.

### FORWARD-CARRY MEMORY
1. **Pattern for TASK-0263 (halo Stage 2 driver promotion)**: when transfer_inject's halo Stage 2 consumer lands, replicate this 5-line shape exactly at driver/src/main.rs:385 — drop apply_halo_inference_advisory in favour of apply_halo_inference, propagate Err via map_err(|e| format!('halo-inference error: {e}'))?. Add a parallel pair of pins in tests/sidecar_halo.rs (mirror task0271_strict_rejects_* / accepts_* naming).
2. **passes::common::iv_diag_policy lift trigger**: defer until BOTH halo and reuse driver calls are strict AND a third caller appears, OR until the diagnostic surface needs centralisation (e.g. error-aggregation across both passes). Single-caller lift is premature.
3. **The advisory entry point is the test escape-hatch.** Do NOT delete apply_reuse_inference_advisory even after every driver path is strict — it's used by determinism-pin tests that need to inspect the full error vector (cycle-87 advisory_collects_all_errors_strict_short_circuits is the canonical example). Same forward-carry applies to apply_halo_inference_advisory when TASK-0263 lands.
<!-- SECTION:NOTES:END -->
