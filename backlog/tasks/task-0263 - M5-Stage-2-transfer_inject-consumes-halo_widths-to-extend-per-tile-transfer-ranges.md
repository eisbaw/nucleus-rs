---
id: TASK-0263
title: >-
  M5 Stage 2: transfer_inject consumes halo_widths to extend per-tile transfer
  ranges
status: In Progress
assignee:
  - '@mped-architect-impl'
created_date: '2026-05-24 01:40'
updated_date: '2026-05-24 09:21'
labels:
  - M5
  - compiler
  - halo
  - transfer
  - stage-2
dependencies:
  - TASK-0275
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Stage 2 of the TASK-0260 halo loop. Stage 1 (TASK-0260, cycle 81) landed halo inference + sidecar persistence. This task wires transfer_inject as the consumer.

## Acceptance criteria
1. transfer_inject reads halo_widths from the ACFG.
2. Each XferPlaceholder whose tile.bounds axis has a non-zero halo entry for the producer/consumer kernel has its lo/hi extended by +/- halo (clamped to source range).
3. New e2e cell 05-stencil/distributed x pthreads-async bit-identical to reference.bin.
4. Driver moves from apply_halo_inference_advisory (lenient) to apply_halo_inference (strict), OR keeps lenient with partition-policy-aware error surfacing. Choose consciously and document.

## Honest scope
- Halo on Mod / data-dependent indices remains rejected (PRD §13). Example 11 step_or_seed still has no distributed schedule.
- Block-pair recovery for partition=blocks2d is the separate Stage 3 (TASK-0264).

## Forward-carry from TASK-0260 cycle 81
- Lenient/strict split exists deliberately. Stage 1 stored a 0 entry for every (kernel, iv) the detector inspected (bare-iv case); Stage 2 must treat 0 as no extension needed.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
FORWARD-CARRY from TASK-0260 cycle-81 review (architect P1 + P2):

When Stage 2 wiring lands (this task), harden the existing test coverage with:

1. **Advisory variant direct test** (architect F-P1): build a multi-error fixture (e.g. one Mod-indexed kernel call AND one strided-access kernel call in two different DataRefs); assert:
   - apply_halo_inference (strict): returns Err on the FIRST error encountered.
   - apply_halo_inference_advisory: returns the FULL error list AND a partial halo_widths map for the unaffected kernel calls (the lenient variant's load-bearing contract).
   Today only the strict path is asserted; the lenient/strict dichotomy is invisible to regression. Stage 2 toggles this in the driver — a regression in either direction MUST surface.

2. **Mod/Div explicit-rejection test** (architect F-P2): the cycle-81 documentation claims example-11 game-of-life's Mod-wrap is rejected by the strict detector; verified by reading prog.algo.nuc line 154-160 + the detector's affine_decompose function but NOT pinned by a dedicated test. Recommend: negative_mod_indexed_rejected fixture (a synthetic kernel call with ) asserting typed HaloInferenceError::NonAffineIndex.

3. **Stage 2 driver decision** (the AC#4 toggle): the cycle-81 driver uses apply_halo_inference_advisory (lenient) to preserve the e2e baseline. Stage 2 MUST NOT keep lenient blanket; once transfer_inject consumes halo, a missing entry for a partition=* axis becomes wrong output. Choose partition-policy-aware fatality: strict on directives that have a partition= consumer; lenient on bare directives. Document the decision + the e2e cell that bites it (the bit-identical 05-stencil/distributed cell IS the new test that promotes the decision).

Cycle 83: transfer_inject extension landed (commit cf2f9ac). For each XferPlaceholder whose source/dest kernel has a non-zero halo entry on the tile axis, the tile lo/hi are extended by ±halo (clamped to source range). Verified by reading the emitted main.rs for 05-stencil/distributed × pthreads-async: each worker receives its extended slice (w0 gets img_in[0..96], w1 gets [64..160], etc. — exactly the row-band + halo on each side).

The codegen is CORRECT. The runtime deadlock that surfaced when the cell was promoted to [[required]] is NOT a transfer_inject bug; it's the partition_rows × sync_inject seam (unequal per-worker iteration counts vs per-iteration barriers — diagnosed under TASK-0266).

Status: In Progress. AC#1/2 met (sidecar consumed; tiles extended). AC#3 (new e2e cell bit-identical) BLOCKED on TASK-0266. AC#4 (lenient → strict driver toggle) DEFERRED — Stage-1 lenient stance preserved for cycle 83 since strict promotion is meaningless until TASK-0266 unblocks.

## Forward-carried from TASK-0271 (cycle 88, 2026-05-24)

TASK-0271 promoted the reuse_inference driver call to STRICT (apply_reuse_inference vs apply_reuse_inference_advisory). The promotion pattern is the precedent for halo when this task closes:

1. **Trigger condition**: promote halo's driver call (currently at nucleus/driver/src/main.rs:385 apply_halo_inference_advisory) to apply_halo_inference once the transfer_inject Stage 2 halo consumer lands. At that point a silently-swallowed typed HaloInferenceError corresponds to a partition-required halo that the backend would silently emit a wrong-output tile for.

2. **5-line shape** (mirror TASK-0271 commit 0a74bea exactly):
   - replace 'apply_halo_inference_advisory' in the use-statement with 'apply_halo_inference'.
   - replace 'let (acfg, halo_errors) = apply_halo_inference_advisory(&linked, acfg); for e in &halo_errors { nucleus_compiler::nuc_trace!(...) }' with 'let acfg = apply_halo_inference(&linked, acfg).map_err(|e| format!("halo-inference error: {e}"))?;'.
   - rewrite the surrounding multi-line comment block to reflect strict policy + cross-link TASK-0263 + TASK-0271 as the reuse precedent.
   - Update halo_inference.rs module docs ('Strict vs advisory entry points') to name the driver as strict consumer; rewrite apply_halo_inference_advisory doc-comment as test-only.

3. **Add two parallel pins** in nucleus/nucleus-compiler/tests/sidecar_halo.rs mirroring tests/sidecar_reuse.rs cycle-88 tail:
   - task0263_strict_rejects_non_affine_halo_body — synthetic non-affine kernel-arg DataRef under a partitioned loop → typed HaloInferenceError.
   - task0263_strict_accepts_shipped_05_stencil_distributed_schedule (or whichever shipped halo-tagged schedule remains valid at promotion time).

4. **CAVEAT — example 11 game-of-life**: per the cycle-82 doc-comment in halo_inference.rs the strict promotion may newly-reject example 11's step_or_seed kernel which reads grid[(t + ITERS) % (ITERS + 1)] (constant modulo wrap that the affine detector cannot fold). VERIFY at promotion time: either (a) the schedule never carries a partition= directive that would activate transfer_inject Stage 2 consumption of halo_widths for the affected iv, in which case strict still passes (no widths needed), OR (b) the affine detector needs to be taught about constant-mod folding, OR (c) the example needs a code change. This is the one shipped corner where (A) strict is NOT obviously safe and (B) partition-policy-aware may be necessary.

5. **DO NOT delete apply_halo_inference_advisory** after promotion — it's the test escape-hatch (mirrors how cycle-88 preserved apply_reuse_inference_advisory for the advisory_collects_all_errors_strict_short_circuits pin in reuse_inference.rs).

CYCLE-89 VERIFICATION (orchestrator, 2026-05-24): the TASK-0271 forward-carry caveat (example 11 step_or_seed Mod-indexed read) is CONFIRMED reachable today.

**Code path verified**: `nucleus/nucleus-compiler/src/passes/halo_inference.rs:361-367` `apply_halo_inference` walks `linked.algo.stmts` UNCONDITIONALLY (no gate on schedule directives, no gate on "is this iv partitioned"). The strict variant short-circuits on the first error from `infer_halo_widths` (line 412).

**Example 11 confirmation**: `nuc-nucleus/examples/11-game-of-life/prog.algo.nuc:156-158` shows `step_or_seed(grid[(t + ITERS) % (ITERS + 1)][(i + N - 1) % N], ...)`. Both `naive.sched.nuc` AND `pipelined.sched.nuc` carry zero `halo`/`partition`/`reuse` directives — the only schedule comment found that mentions partition is at pipelined.sched.nuc:87-88: "No `partition=workers` on the inner `i` loop. The grid is small (N=32) and partitioning the spatial axis is example future scope."

**Conclusion**: a naive TASK-0271 mirror (replace `apply_halo_inference_advisory` with `apply_halo_inference` + `.map_err(...)?`) WOULD newly-reject example 11, breaking the e2e matrix on both example-11 cells. (A) strict for halo is NOT safe today.

## Updated path forward for AC#4 (lenient → strict driver toggle)

The cycle-88 TASK-0271 forward-carry was prescient — the choice for halo IS (B) partition-policy-aware, NOT (A) strict. Specifically:

**Proposed (B) rule**: a `HaloInferenceError` for `(kernel, iv)` is FATAL iff the iv at the kernel-call site carries a `partition=` directive in the schedule that would activate transfer_inject's halo-extending consumer (cf. TASK-0263's cf2f9ac). Otherwise advisory (current behaviour).

The per-error decision needs the call-site-to-iv-scope mapping (which Repeat encloses this OpId) crossed with `LinkedIR.sched.loops` `partition=` lookup.

## AC#4 refinement

AC#4 (lenient → strict driver toggle) now precisely depends on either:
1. **TASK-0275** (to be filed if/when this work starts): implement (B) partition-policy-aware fatality for halo_inference at the driver call site. Per-error scope lookup + partition= check.
2. **TASK-0263.bis** (alternative): teach the affine detector constant-modulo folding (`(x + K) % M` where both K, M are compile-time constants → bounded scope). Would let strict pass on example 11 without policy lift. Likely 100-200 LoC of detector work + tests.
3. **(C)** narrow change to example 11: refactor `step_or_seed` to not use Mod-indexing. Smallest code change but loses the kernel's expressiveness; example 11's whole point is exercising a cyclic-buffer access pattern.

Recommendation: option 1 ((B) partition-policy-aware) — the principled fix, scales to other future cases where the strict promotion would over-eagerly reject affine-detector-defeating shapes that no schedule actually depends on. File TASK-0275 when this work starts.

UNTIL AC#4 lands, the cf2f9ac transfer_inject extension (cycle 83) is good code with its consumer halo_widths populated by the lenient driver. The e2e 92/77/0/15/0 baseline is preserved.

The cycle-87 review item 1 (driver promotion for halo) is REFINED: do NOT mirror TASK-0271's (A) strict pattern; halo genuinely needs (B). TASK-0271's rationale was specific to reuse (Tier 1 marker consumer makes every slot consumed).
<!-- SECTION:NOTES:END -->
