---
id: TASK-0315
title: >-
  Silent-bypass guard for order_halo_strip_bounds_by_data_dim default-order
  fall-back (TASK-0306 cycle-133 architect P3-2)
status: To Do
assignee: []
created_date: '2026-05-25 09:17'
labels:
  - compiler
  - hardening
  - transfer_inject
  - forward-carried-from-TASK-0306
dependencies:
  - TASK-0306
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0306 cycle 133 added the helper `order_halo_strip_bounds_by_data_dim` (transfer_inject.rs:1990+) with a default-order fall-back for synthetic test fixtures built via `DataflowEdge::new` (empty `data_in_access` indices ⇒ `data_dim_iv_map[data]` is `Some(empty)` / `None`). The fall-back is necessary to preserve halo_strip_synth.rs's positive_3x3 / positive_2x2 / determinism / placement tests.

## Risk

The cycle-133 axis-mapping defense is SILENTLY DISABLED on the default-order branch. A future engineer extending halo_strip_synth.rs who accidentally uses `DataflowEdge::new` (instead of the new `build_2x2_acfg_with_indexed_access` helper) would write a test that goes through default-order — the test would pass even if the helper itself silently regressed.

## Acceptance criteria

1. Add a `nuc_trace!` log line on the default-order branch (project diagnostics convention per project-diagnostics-convention.md) so the path is observable when `NUC_TRACE=1`.
2. (Alternative or additive) Add a property test that constructs a fixture with non-empty indexed accesses and asserts the helper does NOT take the default-order path.
3. (Alternative or additive) Promote the `None` branch of the per_dim lookup to `#[cfg(debug_assertions)] panic!` — `None` is truly unreachable in production (every Operation reads data via accesses that get recorded by `walk_data_dim_iv_map`).

## Honest scope

LOW priority. Today's defense (cycle-133 helper) is correct; this task adds an observation-layer so a future regression in the helper would not be masked by the default-order fall-back in synthetic tests.

## Forward-carried from TASK-0306 cycle 133 architect P3-2 (read-only review of commit 7f10a80)
<!-- SECTION:DESCRIPTION:END -->
