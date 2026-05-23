---
id: TASK-0249
title: >-
  partition=rows and partition=blocks2d are silently inert (no downstream
  consumer)
status: To Do
assignee: []
created_date: '2026-05-23 07:40'
labels:
  - compiler
  - partition
  - silent-drop
  - honesty
  - M3
dependencies: []
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Finding (cycle 65, during TASK-0144.02 sizing)

Audit of the partition handling in nucleus/compiler/src showed that of the three PartitionKind variants:

- **PartitionKind::Workers**: consumed by passes/partition_workers.rs (TASK-0212). Real semantics, real codegen, exercised in 13-cnn-inference/batch_parallel.
- **PartitionKind::Rows**: parsed by sched/parser.rs:573, lowered to ResolvedLoopOption::Partition(PartitionKind::Rows) by sched/lower.rs:1095, then NEVER read by any pass. The 05-stencil/distributed live schedule has `loop y : partition=rows;` (line 19) which today does NOTHING beyond being accepted.
- **PartitionKind::Blocks2d**: same — parsed, lowered, never consumed.

passes/partition_workers.rs:40 actually admits this in a header comment: "partition=rows / partition=blocks2d are orthogonal grammars handled by sibling passes (not yet filed)." — so the gap was known but no task captured it.

## Why this is a recurring-failure-class issue

This is the silent-drop pattern that memory feedback-comment-doc-lie-recurring.md tracks. A schedule writes `partition=rows` and the compiler accepts it silently. PRD §6.3.3 "bad combinations rejected at compile time, not at runtime" applies symmetrically — "silently accepted but does nothing" is the same kind of compile-time landmine as "bad combination accepted".

## Why this matters for TASK-0144.02

TASK-0144.02 ("partition=blocks2d on non-2D loop nest: reject at sched-lower") presupposes Blocks2d is a real consumer. Today rejecting non-2D nests while accepting 2D-nest Blocks2d would still leave the 2D-nest case as a silent no-op. The narrower fix (reject non-2D only) does NOT close the actual silent-drop. .02 should depend on this task.

## Recommended approaches (pick one before implementing)

(a) **Implement consumers** for Rows and Blocks2d (real partition semantics). High value, large scope — separate sibling passes to partition_workers.rs.

(b) **Reject as UnsupportedPartitionKind** at sched-lower with a typed error that names Rows/Blocks2d as not-yet-implemented. Forces user to choose `partition=workers` until consumers land. Note: breaks 05-stencil/distributed.sched.nuc — its `partition=rows` directive would need to either go away (it is inert today so this is a no-op behaviourally) or migrate to `partition=workers` (similar role).

(c) **Lint-warning + e2e probe** that fires when Rows/Blocks2d appears but is unused. Honest middle ground; complements (a) and (b).

## Acceptance criteria

- [ ] #1 Decision recorded: (a), (b), or (c).
- [ ] #2 Implementation lands per the chosen path with a precise typed signal (no silent acceptance).
- [ ] #3 05-stencil/distributed handled (either migrated, deprecated, or its `partition=rows` made effective).
- [ ] #4 sched_lower or sibling-pass test asserts the new behaviour (positive AND negative cases).
- [ ] #5 PRD §6.3.3 cited; partition_workers.rs:40 caveat-comment updated to reflect the new state.

## Dependencies

- Feeds: TASK-0144.02 (which becomes meaningful only after Blocks2d has real semantics or is rejected loudly).
<!-- SECTION:DESCRIPTION:END -->
