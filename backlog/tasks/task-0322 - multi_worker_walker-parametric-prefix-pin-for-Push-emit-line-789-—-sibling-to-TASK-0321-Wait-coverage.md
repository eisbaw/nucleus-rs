---
id: TASK-0322
title: >-
  multi_worker_walker: parametric prefix pin for Push emit (line 789) — sibling
  to TASK-0321 Wait coverage
status: In Progress
assignee:
  - '@mark'
created_date: '2026-05-25 12:05'
updated_date: '2026-05-25 12:12'
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

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Cycle 141 plan:

1. **Placement decision (TASK-0322 AC#3)**: Add task0322_-prefixed test to
   nucleus/backend-common/tests/wait_assign_slice.rs immediately after
   task0321_rendezvous_prefix_substituted_in_2d_row_loop_arm (line 659).
   Reuse make_minimal_tables + one_pair helpers. The "Wait-naming
   mismatch" concern in AC#3 is real but mild — TASK-0321 already lives
   here and shares the exact same prefix-substitution machinery; cost of
   a new push_emit_prefix.rs file would be helper duplication
   (make_minimal_tables, RendezvousIds/PairTiles type aliases, one_pair).
   Mitigation: small docstring patch to the file module-doc acknowledging
   that wait_assign_slice.rs now also covers Push-side prefix-substitution
   pins (TASK-0321/TASK-0322) on the same render_worker_events
   machinery, not just receiver-side Wait shapes. This preserves
   sibling-adjacency for the prefix-substitution sweep and avoids
   helper duplication.

2. **Test body**: parametric loop over rendezvous_prefix in
   {ring, slot, chan}. Construct Event::Push { dst: WorkerId(1), data,
   tile, seq }. Render on WorkerId(0) (the sender; WorkerId(1)="host"
   is the receiver named by make_minimal_tables). Use the SAME data /
   tile / rid shape as TASK-0321 (DataId(7) "img_out" [16,16], 2D tile
   [(y, 1..8), (x, 1..8)], rid=12) so the two tests differ only in the
   event type — sibling-adjacency by construction. Asserts:

   - AC#1: The line "PREFIX_12.push(img_out.clone());" is present, with
     PREFIX being each of the three substituted values in turn.
   - AC#2: For each of the OTHER two prefixes, the substring
     "OTHER_12.push(" is NOT present (defensive — catches a hardcoded
     prefix regression in the Push branch).
   - Sanity: the "// send `img_out` to host" comment substring is
     present (confirms Push branch was entered, not falling through to
     some other event handler).

3. **Verification**: Run inside nix develop shell. The cheap commit
   gate per CLAUDE.md:
     nix develop --command bash -c "just build && just clippy && just test && just test-release && just e2e"
   Expect dev test count = 874 (+1 from cycle-140 baseline 873) and
   e2e 108/92/0/16/0 baseline preserved exactly (pure-additive test;
   no source code changes outside the test file).

4. **Commit**: matching cycle-140 style:
   "backend-common tests + tracker: TASK-0322 cycle 141 — parametric
   Push-prefix sibling pin (multi_worker_walker.rs:789)"
   Body covers AC closure + sibling-pair completion narrative + gate
   numbers.

5. **Review gate**: Parallel qa-test-runner + mped-architect read-only
   on the cycle's commit range. Apply fold-back in-thread if findings
   emerge; file follow-ups for anything beyond scope. Mark Done only
   after both subagents return GO and AC#1 + AC#3 are met (AC#2 was
   already deferred at TASK-0321 cycle 140 — explicit honest-scope).
<!-- SECTION:PLAN:END -->
