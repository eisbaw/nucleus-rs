---
id: TASK-0303
title: >-
  Pin halo_widths narratives on 05-stencil/distributed-2d and
  07-matmul/distributed (TASK-0299 sibling-sweep)
status: To Do
assignee: []
created_date: '2026-05-25 02:45'
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
