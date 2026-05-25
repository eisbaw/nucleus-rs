---
id: TASK-0316
title: >-
  Backend-side wait_slice round-trip test for inner-leading / non-prefix
  halo-strip tile (TASK-0306 cycle-133 architect P3-3)
status: In Progress
assignee:
  - '@mark'
created_date: '2026-05-25 09:17'
updated_date: '2026-05-25 10:39'
labels:
  - M6
  - compiler
  - backend-common
  - wait_slice
  - test-coverage
  - forward-carried-from-TASK-0306
dependencies:
  - TASK-0306
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0306 cycle 133 changed the halo-strip emit order to match wait_slice's `bounds[i] ↔ data.dim[i]` convention for two latent shapes (inner-axis-leading data layout + non-prefix data layout). The new pinning tests (task0306_ac3/ac4/ac5 in halo_strip_synth.rs) verify the transfer_inject OUTPUT (XferPlaceholder tile bounds), but NOT the downstream wait_slice consumption end-to-end.

## What's missing

A backend-side codegen test that:
1. Constructs an inner-leading or non-prefix synthetic shape (e.g. via the cycle-133 `build_2x2_acfg_with_indexed_access` fixture).
2. Drives backend codegen (pthreads-sync / pthreads-async / mp-tcp-bufsync / mp-tcp-event).
3. Asserts that the emitted slice-paste source code is correct (the bounds map to the right data dimensions).

## Why not blocking TASK-0306

No shipped schedule today constructs either shape — every shipped 05/06/07 distributed-{,2d} cell uses outer-leading [outer_iv][inner_iv] data layouts. The cycle-133 helper is a defensive improvement against future M6+ schedules; the backend-side round-trip pin is a defense in depth that becomes load-bearing only when such a schedule exists.

## Acceptance criteria

1. New backend-side test fixture (in backend-common/tests or one of the backends/*/tests directories) that constructs the synthetic ACFG with cycle-133 fixture builder, runs backend codegen via render_wait_assign or equivalent, and asserts slice-paste correctness for an inner-leading layout.
2. Same shape pinning for non-prefix layout (whole-array drop case): asserts backend codegen emits whole-array copy when bounds is empty.
3. Bit-identical preservation: existing M5 cells unaffected (e2e 108/92/0/16/0 baseline preserved).

## Honest scope

LOW priority. Trigger: a future M6+ schedule that constructs either latent shape, OR a fault-injection-style defensive coverage cycle. Per architect P3-3 review (TASK-0306 cycle 133): the cycle-133 fix is currently inspection-correct + transfer_inject-output-test-covered; the backend-side round-trip remains a gap to be filled when the trigger arises.

## Forward-carried from TASK-0306 cycle 133 architect P3-3 (read-only review of commit 7f10a80)
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 136 scope addendum (orchestrator)

AC#1 text mandates 'constructs the synthetic ACFG **with cycle-133 fixture builder**, runs backend codegen via render_wait_assign or equivalent'. Cycle-136 implementation deviates: feeds render_wait_assign DIRECTLY with the IterTile shape the cycle-133 helper would emit, using backend-common's established make_minimal_tables + one_pair test fixture pattern (wait_assign_slice.rs precedent set under TASK-0117 + TASK-0294).

Rationale for the scope deviation:
1. backend-common does NOT import nucleus-compiler pass machinery (only the contract types Event/Sidecar/NameTables). Pulling inject_transfers into backend-common test scope would require a cross-test-crate fixture replicator (build_2x2_acfg_with_indexed_access lives in nucleus-compiler/tests/halo_strip_synth.rs, not importable by another crate's tests).
2. The producer-side cycle-133 helper output is already pinned end-to-end via inject_transfers by task0306_ac3/ac4/ac5 in nucleus-compiler/tests/halo_strip_synth.rs:840,921,970. Re-running inject_transfers in backend-common would duplicate that coverage.
3. The cycle-136 test pins what was actually missing: the BACKEND-SIDE positional bounds[i] ↔ ty.dims[i] contract that wait_slice silently relies on. A wait_slice refactor that drops positional semantics would not be caught by the producer-side pins.

The deviation tightens AC#1 from 'round-trip' (which the test does NOT do — it does not call inject_transfers) to 'backend-side consumer pin for the cycle-133 helper's positional output contract'. AC#2 + AC#3 satisfied as written.
<!-- SECTION:NOTES:END -->
