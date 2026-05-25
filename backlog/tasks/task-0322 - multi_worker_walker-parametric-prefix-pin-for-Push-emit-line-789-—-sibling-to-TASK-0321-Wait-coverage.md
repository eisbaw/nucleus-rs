---
id: TASK-0322
title: >-
  multi_worker_walker: parametric prefix pin for Push emit (line 789) — sibling
  to TASK-0321 Wait coverage
status: To Do
assignee: []
created_date: '2026-05-25 12:05'
labels:
  - backend-common
  - multi-worker-walker
  - test-coverage
  - silent-sibling
  - forward-carried-from-TASK-0321
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0321 cycle 140 closed the parametric `rendezvous_prefix` pin for the **Wait-side** substitution site at `nucleus/backend-common/src/multi_worker_walker.rs:809` (`{prefix}{rendezvous_prefix}_{rid}.wait()`). Architect's cycle-140 P2 silent-sibling sweep found a structurally-identical **Push-side** substitution at line 789:

```rust
"{pad}{prefix}{rendezvous_prefix}_{rid}.push({name}.clone()); // send `{name}` to {to}"
```

A regression that hardcoded `"ring_"` at line 789 would silently break partition=blocks2d Push emit for pthreads-sync, mp-tcp-event, and any future render_worker_events-using backend with a non-`"ring"` prefix. The TASK-0321 test does NOT cover this site because it constructs only an `Event::Wait` (drives the Wait codepath only).

## Cycle-140 architect framing

From the cycle-140 sweep: "Both lines are the same `{prefix}{rendezvous_prefix}_{rid}.<op>(...)` shape; both serve the same partition=blocks2d codegen. This is exactly the cycle-128 meta-rule firing for the third time this session."

## Acceptance criteria

1. Add a sibling test (or extend `task0321_rendezvous_prefix_substituted_in_2d_row_loop_arm`) in `nucleus/backend-common/tests/`. Construct an `Event::Push` and assert the prefix substitution at line 789 emits `{prefix}_{rid}.push(...)` correctly across `{"ring", "slot", "chan"}`.
2. Defensive: assert the wrong prefix substrings are NOT present (mirror the TASK-0321 two-sided pin).
3. If `wait_assign_slice.rs` is the wrong home for a Push-side pin (Wait-naming mismatch), create or use a sibling test file (`push_emit_prefix.rs` or extend an existing `multi_worker_*.rs` test file).

## Honest scope

- LOW priority. Same risk profile as TASK-0321: defensive coverage, no current defect. The production substitution machinery has no prefix-conditional branches.
- Cost: small (one test mirroring the TASK-0321 shape, swapping `Event::Wait` → `Event::Push`).

## Cross-reference

- TASK-0321 cycle 140 final summary (architect P2).
- nucleus/backend-common/src/multi_worker_walker.rs:789 (the uncovered substitution site).
- nucleus/backend-common/src/multi_worker_walker.rs:809 (the Wait site, now covered).
- MEMORY.md `feedback-silent-sibling-defect` (the cycle-128 meta-rule that fired three times in session ending cycle 140).
<!-- SECTION:DESCRIPTION:END -->
