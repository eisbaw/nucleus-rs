---
id: TASK-0321
title: >-
  wait_assign_slice: parametric 2D-tile pin across all 4 rendezvous_prefix
  values (TASK-0295 AC#3 gap)
status: To Do
assignee: []
created_date: '2026-05-25 11:51'
labels:
  - backend-common
  - multi-worker-walker
  - test-coverage
  - forward-carried-from-TASK-0295
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0295 cycle 139 audit (AC#3): all 2D-tile slice-paste tests in `nucleus/backend-common/tests/wait_assign_slice.rs` use the shared `render_one_wait` helper, which hardcodes `rendezvous_prefix: "ring"` (pthreads-async-only).

The four tier-1 backends use four distinct prefixes:
- pthreads-sync: `"slot"` (backends/pthreads-sync/src/multi_worker.rs:536)
- pthreads-async: `"ring"` (backends/pthreads-async/src/multi_worker.rs:516)
- mp-tcp-event: `"chan"` (backends/mp-tcp-event/src/multi_worker.rs:493)
- mp-tcp-bufsync: bypasses `render_worker_events`; calls `render_wait_assign` directly with `rhs = decode_expr(ty)` (backends/mp-tcp-bufsync/src/lib.rs:1196). No prefix involved on this path.

Prefix substitution machinery: a single `format!("{prefix}{rendezvous_prefix}_{rid}.wait()")` at `nucleus/backend-common/src/multi_worker_walker.rs:809`. No prefix-conditional branches in the 2D row-loop dispatch — substitution is rendezvous_prefix-agnostic by construction.

## Risk

A future refactor that hardcoded `"ring_"` inside the 2D row-loop arm (or the `_tmp = X.wait()` builder) would pass the existing test (which feeds prefix=`"ring"`) and ship undetected. pthreads-sync + mp-tcp-event would silently emit wrong rendezvous identifiers for partition=blocks2d schedules.

## Acceptance criteria

1. Parameterise `rows_2d_slice_paste_for_partition_blocks2d` (or add a sibling test) over `{ "ring", "slot", "chan" }`. Assert each emits `{prefix}_{rid}.wait()` with the corresponding prefix.
2. Optionally extend the same parameterisation to `task0316_inner_axis_leading_layout_emits_against_dim0` (the inner-axis-leading 2D pin) and `task0316_non_prefix_layout_empty_bounds_consumer_pin`. Same machinery, three additional asserts each.
3. Document the rationale in test docstring (cycle-139 TASK-0295 AC#3 gap closure).

## Honest scope

- LOW priority. Defensive coverage — no current defect. The render_worker_events machinery has no prefix-conditional branches today.
- Trigger: when adding any new 2D-tile arm or refactoring the rendezvous substitution path. Doing it now also closes a defensive-coverage gap proactively.
- Cost: small (parametric loop or a sibling helper accepting prefix).

## Cross-reference

- TASK-0295 AC#3 (the audit that surfaced this).
- nucleus/backend-common/tests/wait_assign_slice.rs (where the new pin lands).
- nucleus/backend-common/src/multi_worker_walker.rs:809 (the prefix-substitution call).
<!-- SECTION:DESCRIPTION:END -->
