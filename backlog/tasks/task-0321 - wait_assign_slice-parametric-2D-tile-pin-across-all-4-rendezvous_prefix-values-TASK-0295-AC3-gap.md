---
id: TASK-0321
title: >-
  wait_assign_slice: parametric 2D-tile pin across all 4 rendezvous_prefix
  values (TASK-0295 AC#3 gap)
status: Done
assignee:
  - '@mark'
created_date: '2026-05-25 11:51'
updated_date: '2026-05-25 12:06'
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 140 implementation summary + architect P2 fold-back disclosure

### What landed

Added `task0321_rendezvous_prefix_substituted_in_2d_row_loop_arm` to `nucleus/backend-common/tests/wait_assign_slice.rs` (lines 571-659). Iterates over `rendezvous_prefix ∈ {"ring", "slot", "chan"}` and asserts:
1. The expected prefix appears in the `_tmp = X.wait()` line of the 2D row-loop arm.
2. The other two prefixes do NOT appear (catches a hardcoded-prefix regression directly).
3. The 2D row-loop shape (`for _y in 1usize..8usize`) is present (sanity that dispatch hit the 2D arm regardless of prefix).

### AC status

- **AC#1**: DONE. The 2D row-loop slice-paste arm at multi_worker_walker.rs:809 (Wait substitution) is now parametrically pinned across the three render_worker_events-using prefixes.
- **AC#2**: DEFERRED. Decided NOT to extend to `task0316_inner_axis_leading_layout_emits_against_dim0` and `task0316_non_prefix_layout_empty_bounds_consumer_pin` — same machinery, marginal additional coverage, scope discipline kept the cycle minimal.
- **AC#3**: DONE (the test docstring documents the cycle-139 audit + cycle-140 closure).

### Architect P2 fold-back — silent-sibling sweep

mped-architect read-only review (GO with P2) found a sibling gap: the Push-side substitution at `multi_worker_walker.rs:789` (`{prefix}{rendezvous_prefix}_{rid}.push(...)`) is structurally identical to the Wait site at line 809 but is NOT covered by this test (which constructs only `Event::Wait`).

This is the cycle-128 meta-rule firing for the THIRD time in this session:
- Cycle 138: missed 3rd dedup site in transfer_inject.rs docstring (Push splice).
- Cycle 139: missed sibling test files in nucleus/backend-common/tests/.
- Cycle 140 (now): missed sibling Push substitution site in multi_worker_walker.rs.

Per honest-failure discipline, the gap is filed as TASK-0322 (sibling Push-side coverage) rather than expanding TASK-0321's scope silently. TASK-0322 is LOW priority — same defensive profile as TASK-0321 — but completes the substitution-site coverage end-to-end.

### Gates

- `just build && just clippy`: green (no warnings under -D warnings).
- `just test` (dev): 873 passed / 0 failed / 3 ignored (was 872; +1 new test).
- `just test-release`: 873/0/3 (matches dev).
- `just e2e`: 108/92/0/16/0 baseline preserved exactly.

### Review gate

- qa-test-runner: GO (independent re-verification of all gate numbers + bite trace + diff scope = pure addition).
- mped-architect: GO with P2 (Push sibling, filed as TASK-0322 — see above).

### Cycle conclusion

AC#1 + AC#3 met; AC#2 honest-deferred (not gamed). The defect-class closure (rendezvous_prefix substitution coverage on the 2D arm) is partial — Wait covered, Push deferred to TASK-0322. Closing TASK-0321 as Done is honest under its scoped wording ("wait_assign_slice: parametric 2D-tile pin") because Push emit lives outside wait_assign_slice's named scope. TASK-0322 carries the sibling end-to-end coverage forward.
<!-- SECTION:NOTES:END -->
