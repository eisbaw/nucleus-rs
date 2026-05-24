---
id: TASK-0271
title: >-
  M5 Stage 2 — reuse_inference driver promotion strict or partition-policy-aware
  (TASK-0265.04)
status: To Do
assignee: []
created_date: '2026-05-24 08:33'
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
