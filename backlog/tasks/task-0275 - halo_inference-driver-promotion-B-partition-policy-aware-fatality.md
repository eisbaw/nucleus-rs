---
id: TASK-0275
title: halo_inference driver promotion (B) partition-policy-aware fatality
status: To Do
assignee: []
created_date: '2026-05-24 09:21'
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

TASK-0271 (cycle 88) promoted the reuse_inference driver call to (A) strict because the Tier 1 marker consumer (TASK-0265) makes every recognised slot consumed; (B) partition-policy-aware degenerated to (A) for reuse. TASK-0263 cycle-89 verification confirmed halo is DIFFERENT: `apply_halo_inference` at halo_inference.rs:361 walks `linked.algo.stmts` UNCONDITIONALLY without gating on schedule directives, AND example 11's `step_or_seed` reads `grid[(t + ITERS) % (ITERS + 1)]` (Mod-index the affine detector cannot fold). A naive (A) strict mirror for halo would newly-reject example 11 — even though no shipped schedule for example 11 carries halo/partition/reuse directives.

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
