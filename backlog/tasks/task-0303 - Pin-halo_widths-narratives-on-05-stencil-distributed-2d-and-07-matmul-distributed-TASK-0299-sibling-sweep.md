---
id: TASK-0303
title: >-
  Pin halo_widths narratives on 05-stencil/distributed-2d and
  07-matmul/distributed (TASK-0299 sibling-sweep)
status: Done
assignee: []
created_date: '2026-05-25 02:45'
updated_date: '2026-05-25 03:02'
labels:
  - M5
  - compiler
  - test-coverage
  - halo_inference
  - comment-doc-lie
  - silent-sibling
  - forward-carried-from-TASK-0299
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0299 cycle 119 added a structural pinning test for the load-bearing halo claim in nuc-nucleus/examples/06-separable-filter/schedules/distributed.sched.nuc:19-21 (halo_widths[hblur_acc][hy] = 0). The cycle-119 architect review-gate identified two structurally identical schedule headers that carry similar load-bearing halo narratives but are NOT yet pinned by an analogous test. This task closes the silent-sibling gap.

## Sibling 1: 05-stencil/distributed-2d.sched.nuc:53

Header claims halo_y = halo_x = 1 (inferred from blur3's 3x3 access). Today an existing test stencil_3x3_produces_halo_one_on_both_axes in nucleus/nucleus-compiler/tests/sidecar_halo.rs:71 covers the 05-stencil naive schedule (halo inference is partition-independent, so the inferred map is the same), but the *narrative claim* lives on the distributed-2d schedule header where it is load-bearing for the halo-strip Push/Wait synthesis pass that the same header attributes to TASK-0289 cycle 114a.

## Sibling 2: 07-matmul/distributed.sched.nuc:25

Header claims 'no halo, no cross-worker carry, no reduction across i'. Equivalent to 'max halo over i is 0 across the algorithm'. Not pinned today; a future kernel-surface change introducing a non-zero i-axis offset (e.g. a[i+1][k] in matmul_acc) would silently lie at the schedule comment while only the e2e bytes would catch the wrong output.

## Acceptance criteria

1. Add a test (in nucleus-compiler/tests/sidecar_halo.rs as a sibling to task0299_*) that loads 05-stencil/prog.algo.nuc + schedules/distributed-2d.sched.nuc and asserts halo_widths[blur3][y] == 1 AND halo_widths[blur3][x] == 1 (the load-bearing claim from distributed-2d.sched.nuc:53). Note this is different from the existing naive-schedule test in that it explicitly loads the distributed-2d schedule so the narrative is tied to that specific header.

2. Add a second test that loads 07-matmul/prog.algo.nuc + schedules/distributed.sched.nuc and asserts the max halo across the i-axis is 0 (the load-bearing claim from distributed.sched.nuc:25). Defensive: also assert the per-kernel max halo across all axes that an inspection of matmul's algorithm should yield 0 (matmul_acc(c[i][j], a[i][k], b[k][j], j, k) reads only at offset 0).

3. Both tests use .unwrap_or(0) on the absent-or-explicit-0 contract degree of freedom, per the TASK-0299 precedent and halo_inference.rs:53-57.

4. Test docstrings name the specific schedule-header line they pin and explain the failure mode (a future kernel-surface change introducing a non-zero offset would fail loud, defending against feedback-comment-doc-lie-recurring on the sibling narratives).

## Honest scope

LOW priority. Pure narrative-pinning hygiene. The e2e bytes already bite on any wrong output today; this task just makes the *narrative* a structural invariant on par with the cycle-119 TASK-0299 precedent. No new code coverage of a previously-untested code path — halo_inference is already covered.

## Cross-references

- TASK-0299 (cycle 119, Done) — the precedent + the architect P2 finding that surfaced this task.
- feedback-comment-doc-lie-recurring memory entry.
- feedback-silent-sibling-defect memory entry (a pin at one site without pinning the structurally-identical siblings is the named pattern this task closes).
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
ORCHESTRATOR CLOSE (cycle 120, 2026-05-25).

Added two structural pinning tests in nucleus/nucleus-compiler/tests/sidecar_halo.rs as siblings to task0299_*:

1. task0303_05_stencil_distributed_2d_halo_widths_pinned_to_one
   - Loads 05-stencil/prog.algo.nuc + schedules/distributed-2d.sched.nuc via the existing lower() helper.
   - Asserts halo_widths[blur3][y] == 1 AND halo_widths[blur3][x] == 1 (the load-bearing claim at distributed-2d.sched.nuc:53).
   - Defends against feedback-comment-doc-lie-recurring on this sibling narrative.

2. task0303_07_matmul_distributed_halo_widths_pinned_to_zero
   - Loads 07-matmul/prog.algo.nuc + schedules/distributed.sched.nuc via the lower() helper.
   - Asserts halo_widths[madd][i] == 0 (the literal i-axis half of the distributed.sched.nuc:25-26 claim) AND defensive max-halo across the whole algorithm == 0.
   - Defends against feedback-comment-doc-lie-recurring on this sibling narrative.

Both tests use .unwrap_or(0) per the halo_inference contract degree of freedom (explicit-0 OR omission permitted; see halo_inference.rs:53-57 + cycle-119 TASK-0299 precedent). Both pin behaviour against future kernel-surface changes that would silently lie at the schedule comment while only e2e bytes catch the wrong output.

Test runs:
- sidecar_halo file: 12 passed (10 pre-cycle-120 + 2 new), 0 failed.
- Pre-commit gate (just build && clippy && test && test-release && e2e): all green.
- E2e baseline unchanged: 104/88/0/16/0 (test-only addition has no production code path).

AC#1 (05-stencil/distributed-2d pin on blur3[y]=1, blur3[x]=1): DONE
AC#2 (07-matmul/distributed pin on max-halo-across-i==0): DONE
AC#3 (.unwrap_or(0) contract degree of freedom): DONE
AC#4 (test docstrings name specific schedule-header lines + failure modes): DONE

Honest scope-narrowings (pre-empting cycle-119 P2.1-class disclosure):
- task0303_05_*: pins ONLY halo_widths values; does NOT pin that the halo-strip Push/Wait synthesis pass (TASK-0289) actually fires on these widths, NOR that transfer_inject extends per-tile transfer ranges by halo=1. The schedule header attributes both behaviours to TASK-0289 cycle 114a; this test pins only the precondition (halo widths), not the consumers. A regression in TASK-0289's strip synthesis with correct halo_widths would NOT trip this test; the e2e bytes catch it iff the strip-synthesis regression changes output.
- task0303_07_*: pins ONLY the halo_widths zero claim; does NOT pin that the cycle-118 TASK-0301 axis-mapping filter (transfer_inject) produces empty bounds for b. That's a different pass. A regression in TASK-0301 with correct halo_widths would NOT trip this test; the existing 07-matmul/distributed × all 4 tier-1 backends e2e cells catch it via wrong bytes.

Cycle-119 architect-disclosure-mechanism-wrong defense: the driver actually uses apply_halo_inference_partition_aware (B) at nucleus/driver/src/main.rs:396; the test calls apply_halo_inference (A, strict) via lower(). A and B agree on clean-affine inputs (both 05-stencil/distributed-2d and 07-matmul/distributed are clean-affine), so the test's coverage is sound across either driver entry-point choice. Cited explicitly so a future reader does NOT have to re-derive the mechanism.
<!-- SECTION:NOTES:END -->
