---
id: TASK-0299
title: >-
  Pinning test for halo_widths[hblur_acc][hy]=0 on 06-separable-filter algorithm
  (TASK-0296 cycle-116 architect P1.1)
status: Done
assignee: []
created_date: '2026-05-25 01:18'
updated_date: '2026-05-25 05:41'
labels:
  - M5
  - compiler
  - test-coverage
  - halo_inference
  - 06-separable-filter
  - forward-carried-from-TASK-0296
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background
TASK-0296 cycle 116 added 06-separable-filter/distributed.sched.nuc with partition=rows on hy. The schedule header asserts (lines ~19-21): "halo inference produces `halo_widths[hblur_acc][hy] = 0` and transfer_inject does NOT extend per-tile transfer ranges." This is CORRECT today by inspection of the algorithm: `hblur_acc(tmp[hy][hx], in_arr[hy][hk], hx, hk)` — `hy` axis is accessed at offset 0 only.

## Risk
The claim is a comment-doc-lie waiting to happen: if a future kernel-surface change introduces a non-zero hy offset (e.g. `in_arr[hy-1][hk]` for a vertical-blur fold), the comment stays stale and the schedule silently mis-claims halo behaviour. The e2e cell would catch wrong output, but the *narrative* in the comment would lie.

## Acceptance criteria
1. Add a test (in nucleus-compiler/tests/halo_inference.rs or nearest sibling) that loads 06-separable-filter/prog.algo.nuc + a partition=rows schedule, runs halo inference, and asserts `halo_widths[hblur_acc][hy] == 0` (and equivalently for vblur_acc[vy]). Pin the claim by structural test.
2. If the algorithm ever changes such that this assertion no longer holds, the schedule comment must be updated in the same commit — the test forces the change to be intentional.

## Honest scope
- LOW priority — defends against a future class of bug, not a current one. The current cell is bit-identical correct.

## Cross-references
- `nuc-nucleus/examples/06-separable-filter/schedules/distributed.sched.nuc:19-21` — the load-bearing comment.
- `nuc-nucleus/examples/06-separable-filter/prog.algo.nuc:100` — the access patterns asserted.
- `nucleus/nucleus-compiler/src/passes/halo_inference.rs` — the inference pass.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
ORCHESTRATOR CLOSE (cycle 119, 2026-05-25).

Added structural pinning test in nucleus/nucleus-compiler/tests/sidecar_halo.rs (task0299_06_separable_filter_distributed_halo_widths_pinned_to_zero) loading 06-separable-filter/prog.algo.nuc + schedules/distributed.sched.nuc through the full lower/link/partition/halo_inference pipeline (the existing per-file lower() helper). Three assertions:

1. halo_widths[hblur_acc][hy] == 0 — the literal claim in distributed.sched.nuc:19-21.
2. halo_widths[vblur_acc][vy] == 0 — mirror property on the vertical pass (pass 2 stays on host today per HONEST SCOPE, but the algorithm claim is symmetric).
3. Defensive max-halo across the WHOLE algorithm == 0 — catches a regression even if the named lookups move.

Contract degree of freedom honoured: the existing halo_inference contract (the "TASK-0305 cycle-122 project decision (Option B)" paragraph in halo_inference.rs (search for "absent ≡ explicit-0")) permits explicit 0-width entry OR omission; the test treats both as 'halo == 0'. The only failure mode it pins is 'halo > 0'. Robust to the implementation toggling between explicit and omitted forms (which is allowed today).

If a future kernel-surface change introduces a non-zero hy offset (e.g. in_arr[hy-1][hk] for a vertical-blur fold), this test fails LOUD and forces distributed.sched.nuc:19-21 to be updated in the same commit. Defends against the feedback-comment-doc-lie-recurring pattern (a sibling pin to TASK-0299's named class).

Test run: 10 passed (the new test + the 9 existing sidecar_halo tests), 0 failed.

AC#1 (structural test on halo_widths[hblur_acc][hy]==0): DONE
AC#2 (forces same-commit update on algorithm change): DONE — test fails LOUD on regression.

Honest limits:
- Test asserts ONLY the rejection of non-zero halo on the inspected ivs. It does NOT assert that the implementation chose explicit-0 vs omission today; if a future PR toggles that representation, the test stays green (correct — that's a contract degree of freedom).
- Test does not exercise the apply_halo_inference_partition_aware (B) entry point. Today the strict (A) entry is what the driver uses on this fixture; if the driver switches to (B) for distributed schedules, this test still pins behaviour because (B) is a superset of (A) on clean affine bodies.

ORCHESTRATOR REVIEW-GATE HARDENING (cycle 119, post-architect-P1 finding).

CORRECTION to the prior note: the line 'Today the strict (A) entry is what the driver uses on this fixture; if the driver switches to (B) for distributed schedules, this test still pins behaviour because (B) is a superset of (A) on clean affine bodies' was FACTUALLY WRONG on the mechanism. The driver ACTUALLY uses apply_halo_inference_partition_aware (B) — see nucleus/driver/src/main.rs:396 — NOT the strict (A) variant. The conclusion (the test pins behaviour either way) happens to be correct (both A and B return the same halo map for clean-affine input on 06-separable-filter), but the stated mechanism was wrong.

This is a textbook feedback-implementer-disclosure-mechanism-wrong instance, ironic given that TASK-0299 itself defends against the sibling feedback-comment-doc-lie-recurring pattern. The architect review-gate caught it cycle 119; orchestrator surfaces it here.

Corrected mechanism statement:
- Driver uses apply_halo_inference_partition_aware (B) on the production path.
- The test calls the lower() helper which calls apply_halo_inference (A, strict).
- Both A and B return the same halo map for the 06-separable-filter clean-affine body (B is a superset of A: it widens the error-handling envelope, not the map-population logic). So the test's halo-widths assertions are sound regardless of which entry point the driver picks. If the driver ever switches BACK to (A), this test still pins behaviour; if a new (C) entry point lands, the test would need to be re-aimed.

P2 architect findings filed/handled separately:
- Two-part schedule-header conjunction: only the first conjunct (halo_widths value) is pinned by this test; the second conjunct (transfer_inject does NOT extend per-tile ranges) requires a different fixture. Test docstring updated to disclose this narrowing.
- Silent-sibling sweep: two other schedule headers (05-stencil/distributed-2d.sched.nuc:53 and 07-matmul/distributed.sched.nuc:25) carry similar load-bearing halo narratives that are not pinned. Filed as TASK-0303 follow-up.
<!-- SECTION:NOTES:END -->
