---
id: TASK-0275
title: halo_inference driver promotion (B) partition-policy-aware fatality
status: Done
assignee:
  - '@mped-architect-impl'
created_date: '2026-05-24 09:21'
updated_date: '2026-05-24 11:35'
labels:
  - M5
  - driver
  - halo
  - stage-2
  - forward-carried-from-TASK-0263
dependencies:
  - TASK-0263
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

Forward-carried from TASK-0263 cycle-89 verification + TASK-0271 cycle-88 forward-carry caveat.

TASK-0271 (cycle 88) promoted the reuse_inference driver call to (A) strict because the Tier 1 marker consumer (TASK-0265) makes every recognised slot consumed; (B) partition-policy-aware degenerated to (A) for reuse. TASK-0263 cycle-89 verification confirmed halo is DIFFERENT: `apply_halo_inference` (search for `fn apply_halo_inference` in halo_inference.rs) walks `linked.algo.stmts` UNCONDITIONALLY without gating on schedule directives, AND example 11's `step_or_seed` reads `grid[(t + ITERS) % (ITERS + 1)]` (Mod-index the affine detector cannot fold). A naive (A) strict mirror for halo would newly-reject example 11 — even though no shipped schedule for example 11 carries halo/partition/reuse directives.

## Why halo needs (B) but reuse didn't

The transfer_inject halo consumer (cf2f9ac, cycle 83) only EXTENDS XferPlaceholder ranges when the `(kernel, iv)` has a non-zero halo width. If the iv is NOT partitioned, transfer_inject doesn't ship halo strips, so a missing halo entry is harmless. The reuse Tier 1 marker, by contrast, fires for EVERY recognised slot regardless of partition — making the consumer universal and (A) strict the right choice.

## Proposed (B) rule

A `HaloInferenceError` for `(kernel, iv)` is FATAL iff the iv at the kernel-call-site carries a `partition=` directive in the schedule that activates transfer_inject's halo-extending consumer. Otherwise advisory (current behaviour).

The per-error decision needs:
1. Map `OpId` (the kernel-call site) → enclosing-Repeat chain → IterVar set.
2. Cross with `LinkedIR.sched.loops[iv].options` for `LoopOption::Partition` (or whatever variant tags the directive).
3. If the iv-axis is partitioned AND `infer_halo_widths` failed for that pair → FATAL.

## Acceptance criteria

1. Driver call at `nucleus/driver/src/main.rs:385` upgrades from blanket-lenient `apply_halo_inference_advisory + nuc_trace!` to either:
   - (a) a new pass-side wrapper `apply_halo_inference_partition_aware(linked, acfg) -> Result<ACFG, HaloInferenceError>` that bakes the per-error scope check in, OR
   - (b) explicit driver-side filter: call advisory + walk the errors vec + escalate the partition-relevant ones.
2. Decision documented in driver comment + the chosen pass module docs.
3. Tests pinning BOTH arms: 
   - non-affine halo on a partitioned iv → FATAL.
   - non-affine halo on an UN-partitioned iv (example 11 shape) → ADVISORY (matches today).
4. e2e 92/77/0/15/0 preserved (example 11's two cells stay PASS).
5. determinism-check GREEN.

## Alternative paths considered

- **Option 2: teach the affine detector constant-modulo folding** (`(x + K) % M` where K, M are compile-time constants). 100-200 LoC of detector work + variants on `infer_halo_widths` + tests. Strict could then pass on example 11 directly. Pro: simpler driver. Con: detector complexity; only addresses the constant-mod shape, not future affine-defeating patterns.
- **Option 3: refactor example 11 to drop Mod-indexing**. Smallest code change but loses the cyclic-buffer kernel pattern the example exists to demonstrate.

This task targets Option 1 (B partition-aware) as the principled fix.

## Coordination

- Unblocks TASK-0263 AC#4 (halo driver lenient → strict).
- Does NOT need to mirror TASK-0271's exact pattern — halo case is genuinely different (transfer_inject consumer is conditional on partition; reuse Tier 1 marker is universal).
- The per-error scope lookup logic may eventually be lifted to `passes::common` IF a third pass needs the same shape — cosmetic at this point.

## Dependencies

- Blocked by: nothing (transfer_inject halo consumer already wired cf2f9ac cycle 83).
- Forward-carried from: TASK-0263 cycle-89 verification of TASK-0271 cycle-88 forward-carry caveat.
- Closes: TASK-0263 AC#4 when landed.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Implementation Plan (orchestrator cycle 96 brief, 2026-05-24)

### Architectural fit
The (B) partition-policy-aware fatality rule needs per-error scope lookup. The HaloInferenceError variants do NOT carry the enclosing-loop iv set (they carry kernel+ref_name+ax_idx only). The walker DOES have scope at error-push time ( is threaded through every walker fn).

Cleanest path = (a) from AC#1: new pass-side wrapper that pairs errors with scopes internally, decides per-error, and returns Result.

### Specific changes

**1. nucleus/nucleus-compiler/src/passes/halo_inference.rs**
   - Change `fn infer_halo_widths` return type from `Vec<HaloInferenceError>` to `Vec<(HaloInferenceError, Vec<String>)>`. Every internal `errors.push(E)` site → `errors.push((E, scope.to_vec()))`. Walker signatures change accordingly (the `scope: &[String]` is already in scope at every push site — verify by grep: lines 571, 655, 678, 694, 704, 723).
   - `apply_halo_inference` (strict): strip scopes from the Err arm.
   - `apply_halo_inference_advisory` (lenient): strip scopes from the returned Vec.
   - NEW `pub fn apply_halo_inference_partition_aware(linked, acfg) -> Result<(ACFG, Vec<HaloInferenceError>), HaloInferenceError>`. For each (error, scope), check `scope.iter().any(|iv| iv_is_partitioned(linked, iv))`; if YES → return Err(e) on the first; if NO → push to advisory vec. Return Ok((committed_acfg, advisory)).
   - NEW private `fn iv_is_partitioned(linked, iv: &str) -> bool`: `linked.sched.loops.get(iv).map(|d| d.options.iter().any(|o| matches!(o, ResolvedLoopOption::Partition(_)))).unwrap_or(false)`.
   - Module docs: extend the 'Strict vs advisory entry points' section with a third 'Partition-policy-aware' bullet. Cross-link TASK-0263 + TASK-0275.

**2. nucleus/nucleus-compiler/src/lib.rs**
   - Re-export `apply_halo_inference_partition_aware` from passes::halo_inference.

**3. nucleus/driver/src/main.rs**
   - Around line 385, replace the lenient call with:
     ```rust
     let (acfg, halo_errors_advisory) = apply_halo_inference_partition_aware(&linked, acfg)
         .map_err(|e| format!("halo-inference error: {e}"))?;
     for e in &halo_errors_advisory {
         nucleus_compiler::nuc_trace!("halo_inference: advisory (no partition= in scope, lowering proceeds): {e}");
     }
     ```
   - Rewrite the surrounding multi-line comment block to document (B) policy + cross-link TASK-0271 reuse precedent + the example-11 step_or_seed reason (Mod-indexed read with no partition= in scope = stays advisory).
   - Update use-statement: `apply_halo_inference_advisory` → `apply_halo_inference_partition_aware`.

**4. nucleus/nucleus-compiler/tests/sidecar_halo.rs** (NEW pinning tests)
   - `task0275_partition_aware_rejects_non_affine_under_partitioned_iv`: synthetic LinkedIR with a kernel call inside a `for y` loop where `sched.loops["y"]` carries `Partition(Workers)`, and the kernel arg has a non-affine index → `apply_halo_inference_partition_aware` returns Err.
   - `task0275_partition_aware_accepts_non_affine_under_unpartitioned_iv`: same shape but sched.loops["y"] has NO Partition option (e.g. only `block=64`) → returns Ok((_, advisory)) with the error in advisory vec, NOT Err.
   - `task0275_partition_aware_accepts_clean_affine_under_partitioned_iv`: clean `grid[y+1]` inside partitioned y-loop → Ok((_, [])) with halo width 1.

### Verification gate (run from `nix develop -c`)
- `just test` — all unit tests pass
- `just e2e` — 92/77/0/15/0 preserved (example 11 cells stay PASS)
- `just determinism-check` — green at 92/77/0/15
- `just fmt-check` — 0 (TASK-0276 owns the global drift; this task must not add new drift)
- `cargo clippy --all-targets -- -D warnings` — clean

### Critical onboarding caveats (forward-carried from prior cycles)
- TASK-0271 cycle-88 precedent shape: `apply_X(...).map_err(|e| format!("...: {e}"))?`. Don't reinvent.
- The `scope: Vec<String>` is outermost-first (collect_from_stmts pushes via clone-and-push in the For arm).
- Do NOT delete `apply_halo_inference_advisory` after promotion — it's the test escape-hatch (mirror cycle-88 reuse pattern).
- Example 11 step_or_seed grid[(t + ITERS) % (ITERS + 1)] is the canonical advisory case. naive.sched.nuc + pipelined.sched.nuc carry ZERO partition= directives, so (B) rule says ADVISORY — cells stay PASS.
- 05-stencil/distributed schedule has `partition=workers` on the y-loop and blur3 is fully affine, so it stays clean (no new fatality).

### Honest scope
- Only AC#1(a), AC#2, AC#3, AC#4 preservation, AC#5 preservation are in this task. AC#3 IS this task. If a synthetic fixture for the test arms is too painful to hand-build, OK to use a real fixture from the test fixtures dir.
- Closes TASK-0263 AC#4 when landed.

## Cycle 96 implementation summary (mped-architect-impl, 2026-05-24)

**Landed in 2 commits:**
- 36eb387 — halo_inference: add partition-policy-aware (B) entry point
- 7b71fd6 — driver: promote halo_inference call to partition-policy-aware

**AC status:**
- AC#1: (a) chosen and landed. New pass entry point `apply_halo_inference_partition_aware(linked, acfg) -> Result<(ACFG, Vec<HaloInferenceError>), HaloInferenceError>` at halo_inference.rs. Re-exported from lib.rs. Driver call upgraded.
- AC#2: Documented in (i) module docs of halo_inference.rs (third bullet under "Strict vs advisory vs partition-policy-aware entry points"), (ii) doc-comment on `apply_halo_inference_partition_aware` itself, (iii) ~30-line driver comment block at the call site, (iv) updated sibling reuse comment block.
- AC#3: Three pinning tests in tests/sidecar_halo.rs:
  - task0275_partition_aware_rejects_non_affine_under_partitioned_iv
  - task0275_partition_aware_accepts_non_affine_under_unpartitioned_iv (the example-11 shape pin)
  - task0275_partition_aware_accepts_clean_affine_under_partitioned_iv (the 05-stencil/distributed shape pin)
- AC#4: e2e 92/77/0/15/0 preserved at both commits (verified twice, post-Commit-1 and post-Commit-2). Example 11's two cells stay PASS as predicted.
- AC#5: determinism-check GREEN at 92/77/0/15.

**Gate numbers (final):**
- `just test`: 67 buckets, all 0 failures.
- `just e2e`: total: 92   pass: 77   fail: 0   skipped: 15   required-fail: 0
- `just determinism-check`: total: 92   pass: 77   fail: 0   skipped: 15
- `cargo clippy --all-targets --workspace -- -D warnings`: clean (1 hit on the new walker return type fixed in-flight via `HaloErrorWithScope` + `HaloMap` type aliases).
- `just fmt-check`: NOT run as a gate per orchestrator brief (TASK-0276 owns the pre-existing global drift); inspected diff manually — no new drift introduced on touched lines.

**Lessons learned (forward-carry candidates):**
- The walker had to thread the `scope: Vec<String>` clone into every error push site (6 sites). The natural data type became `Vec<(HaloInferenceError, Vec<String>)>` which clippy flags as `type_complexity`. Introduce a type alias EARLY (here: `HaloErrorWithScope` + `HaloMap`) rather than fight a too_many_arguments-style clippy bite mid-refactor. The reuse_inference pass (TASK-0261/0271) doesn't have this shape because its errors carry the iv-name directly in the variant payload; halo's variants do not (they carry kernel+ref_name+ax_idx only) — that asymmetry is structural and was not worth changing for this task.
- The (B) predicate `iv_is_partitioned(linked, iv)` is the canonical shape for a per-error partition-policy lookup. If a third pass needs it (none today), lift to passes::common — the call site is 6 lines.
- Determinism-check did not regress: the new advisory bucket is a Vec built in walker-source-order, so deterministic by construction.
- ZERO doc-lie defects shipped (cycle-93 forward-carried sub-recurring): every cross-reference between halo and reuse comment blocks was re-grepped post-edit and verified.

**Closes:** TASK-0263 AC#4 (lenient → strict driver toggle).

**No new follow-up tasks filed** — the partition-aware policy is end-state for the halo driver promotion; the remaining halo work (block-pair recovery TASK-0264, the 05-stencil/distributed deadlock TASK-0266) is unrelated to the lenient/strict policy choice.

## Cycle 96 review-hardening summary (orchestrator, 2026-05-24)

Parallel review gate (qa-test-runner + mped-architect, read-only) on commits 36eb387/7b71fd6: both **GO**.

### Architect P2 (material): test-name vs AC#3 literal
Two of three original pins (`_rejects_non_affine_under_partitioned_iv` and `_accepts_non_affine_under_unpartitioned_iv`) used `y*2` — that is *strided* (coefficient != 1, technically affine), not `NonAffineIndex`. The AC#3 wording AND the example-11 motivation are about `NonAffineIndex` (Mod-shape the affine detector unconditionally rejects per `passes::common` contract).

**Fix landed in commit 1cdf345**: rename strided pins to `_strided_*` + add true `_non_affine_*` pins using `y % 4` (the example-11 `step_or_seed` shape under a hypothetical partition=). Both arms (FATAL under partition=, ADVISORY without) pinned.

### Architect P3 (cheap): partition= variant coverage
`iv_is_partitioned` matches `Partition(_)` (every `PartitionKind` variant) but only `Workers` was test-covered. A regression that narrowed the arm to `Workers`-only would silently let `partition=rows` schedules emit wrong tiles.

**Fix landed in commit 1cdf345**: `task0275_partition_aware_rejects_strided_under_partition_rows` pin locks the every-variant contract.

### Architect P3 + QA observation: driver error string
`"halo-inference error: {e}"` does not name the (B) policy that made the error fatal; users would have to read the 30-line driver-comment block to understand why their formerly-advisory body is now a compile error.

**Fix landed in commit 1cdf345**: rewritten to `"halo-inference error (under partitioned iv): {e}"`.

### Gate after hardening (1cdf345 + the cycle-96 pair)
- `cargo test -p nucleus-compiler --test sidecar_halo`: **9/9 PASS** (5 TASK-0275 pins + 4 prior halo tests)
- `just e2e`: **total: 92  pass: 77  fail: 0  skipped: 15  required-fail: 0** byte-identical
- `just determinism-check`: **total: 92  pass: 77  fail: 0  skipped: 15** GREEN
- `just clippy`: clean

### Findings deferred (not actionable now)
- QA P3: `apply_halo_inference_advisory` remains a public re-export but driver no longer calls it. Retained per cycle-88 pattern (test escape hatch); future dead-public-API sweep can revisit.
- Architect P3: `apply_halo_inference_partition_aware` returns Err on the first scope-fatal error, discarding any already-collected advisory errors. Consistent with strict variant's first-error contract; documented if a multi-error mode is needed later.
- Architect P3: 5 backlog md files reference `UnknownIterVarInScope` (the cycle-95 rename target), each annotated `[CYCLE-95 UPDATE]` — historical, not load-bearing. Skipping.

### Cycle 96 outcome
TASK-0275 (B) partition-policy-aware halo driver promotion: **Done, review-GO**.
- TASK-0263 AC#4 (lenient → strict driver toggle): CLOSED.
- Commit chain: 36eb387 (pass-side + 3 pins) → 7b71fd6 (driver promotion) → 4bb5db6 (tracker Done + forward-carry) → 1cdf345 (review-hardening: rename strided pins + add genuine non-affine pins + partition=rows pin + enriched error string).
- Forward-carried lessons to TASK-0263, TASK-0269, TASK-0270 confirmed accurate by reviewer.
<!-- SECTION:NOTES:END -->
