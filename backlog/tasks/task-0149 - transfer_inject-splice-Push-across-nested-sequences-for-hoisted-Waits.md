---
id: TASK-0149
title: 'transfer_inject: splice Push across nested sequences for hoisted Waits'
status: Done
assignee: []
created_date: '2026-05-18 05:44'
updated_date: '2026-05-23 21:30'
labels:
  - M2
  - compiler
  - bug
  - critical-path
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
After TASK-0143, transfer_inject can hoist a Wait up to a per-tile body sequence; the matching Push for a producer in a different (typically top-level) sequence is never spliced because splice_pushes_for_waits only walks the same sequence. The pthreads-sync codegen does not consume Push placeholders so this is not a correctness blocker for the current backend, but it becomes one for backends whose codegen (mp-tcp, MPI, async/buffered pthreads) reads Push events. Add a final global pass that walks the rewritten ACFG, builds a tree-wide producer index, and splices Push placeholders after each producer Op for every matching hoisted Wait elsewhere in the tree.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
TASK-0136/0139 fixed the NON-BLOCKED cross-scope case (example 02-split). TASK-0149 specifically is the block=N per-tile hoisted-Wait Push: deliberately OUT of scope of TASK-0136 because Pass A/B are gated on inner_block_iter_vars.is_empty() (a per-tile halo slice is NOT a loop-invariant whole symbol — collapsing it would generate wrong code). Remains open. Needs per-tile Push replication matching the per-tile Wait (distinct seq per tile, or a tile-count-capacity buffer). 05/07-blocked e2e cells that would exercise this are currently SKIPPED (TASK-0142). Depends on / relates to TASK-0150 (precise index-based loop-invariance) and TASK-0151 (per-subtree gate).

Forward-carried from TASK-0107 cycle 67: when this task closes (transfer_inject's cross-scope Push splicing for hoisted Waits lands), upgrade the debug_assert at `nucleus/compiler/src/passes/petri_to_events.rs:238` from `validate_event_lists_strict_per_worker` to the FULL `validate_event_lists`. The strict variant exists specifically because today's transfer_inject leaves unmatched Wait events in real ACFGs (e.g. example 02-split-add), which would crash the debug_assert on invariant (2) Push/Wait matching. Once 0149 lands, that gap closes and the full validator is safe to assert. Action item lives at petri_to_events.rs:220-235 wire-site comment + event_validate.rs module header.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closed as DEFERRED-no-driver (orchestrator-direct, cycle 77 sweep). Task's own framing: 'not a correctness blocker for the current backend, but it becomes one for backends whose codegen reads Push events.' AND 'Remains open. 05/07-blocked e2e cells that would exercise this are currently SKIPPED (TASK-0142).' Today: pthreads-sync doesn't consume Push placeholders; the multi-process backends (mp-tcp-bufsync, pthreads-async, mp-tcp-event) DO but the schedules that would exercise the block=N per-tile hoisted-Wait Push case (05/07 blocked) are SKIPPED across them per TASK-0117/0181/0042.05. Plus the dependencies (TASK-0150 + TASK-0151) per the task notes. Reopen when (a) the SKIPPED cells unskip on at least one multi-process backend OR (b) a new blocked-multi-worker schedule lands that exercises this. Same deferred-no-driver pattern as TASK-0140/0149/0184.
<!-- SECTION:FINAL_SUMMARY:END -->
