---
id: TASK-0249
title: >-
  partition=rows and partition=blocks2d are silently inert (no downstream
  consumer)
status: Done
assignee:
  - mark
created_date: '2026-05-23 07:40'
updated_date: '2026-05-23 14:50'
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

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 #1 #1 Decision recorded: (a), (b), or (c).
- [x] #2 #2 #2 Implementation lands per the chosen path with a precise typed signal (no silent acceptance).
- [x] #3 #3 #3 05-stencil/distributed handled (either migrated, deprecated, or its `partition=rows` made effective).
- [x] #4 #4 #4 sched_lower or sibling-pass test asserts the new behaviour (positive AND negative cases).
- [x] #5 #5 #5 PRD §6.3.3 cited; partition_workers.rs:40 caveat-comment updated to reflect the new state.

## Dependencies

- Feeds: TASK-0144.02 (which becomes meaningful only after Blocks2d has real semantics or is rejected loudly).
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Implement Approach (b) — typed rejection at sched-lower.

Steps:
1. Add `SchedLowerErrorKind::UnsupportedPartitionKind { var: String, kind: PartitionKind }` in nucleus/compiler/src/sched/ir.rs mirroring `UnitPipelineOption` shape. Doc-comment names what is rejected (Rows / Blocks2d), cites TASK-0249 + PRD §6.3.3, explains the silent-drop motivation, and says how to fix (use `partition=workers` or omit).
2. Add Display impl for the new variant in sched/ir.rs alongside `UnitPipelineOption`. Message is actionable and names the loop var + partition kind.
3. Wire the rejection in sched/lower.rs:1095 — match on PartitionKind: Workers continues to lower; Rows / Blocks2d return the new error.
4. Add the row to the classification-table doc-comment in sched/lower.rs:159 (`Independent | yes | never`).
5. Update passes/partition_workers.rs:40 caveat comment to reflect new state (rejected at sched-lower).
6. Migrate nuc-nucleus/examples/05-stencil/schedules/distributed.sched.nuc:19 by removing the inert `loop y : partition=rows;` line and inserting a comment explaining the deletion + naming the follow-up task. Update the sched_parser / sched_lower / link tests that pin the loop count.
7. Add negative tests (Rows + Blocks2d -> UnsupportedPartitionKind) and positive smoke (Workers still lowers) in nucleus/compiler/tests/sched_lower.rs alongside the UnitPipelineOption test.
8. File follow-up task: "TASK-0249 follow-up: decide whether 05-stencil/distributed should partition the y-loop across {w0..w3} via partition=workers".
9. Run full verification gate (just ci). Bit-identical regressions are hard failures.
10. Update memory note project-partition-silent-drop.md.

Acceptance criteria mapping:
- AC#1 Decision recorded: (b) — see notes.
- AC#2 Implementation: steps 1-3 above.
- AC#3 05-stencil/distributed handled: step 6 (deletion + follow-up).
- AC#4 Tests: step 7 (positive + negative).
- AC#5 PRD §6.3.3 cited + partition_workers.rs:40 updated: steps 1+5.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Cycle 70 (TASK-0249 implementation):

- Decision: Approach (b) — typed reject at sched-lower. Smallest, most
  honest, MPED-aligned change; matches `UnitPipelineOption` / `Unroll`/
  `Vectorize`NotDivisibleByBlock precedent.
- Variant: `SchedLowerErrorKind::UnsupportedPartitionKind { var, kind }`.
- Wire site: `sched/lower.rs::lower_loop_option`, the `LoopOption::Partition(k)` arm.
  Match is exhaustive on PartitionKind so a future variant cannot silently
  fall through.
- Classification: Independent | yes | never (row added to
  `sched/lower.rs:159` table).
- 05-stencil/distributed migration: removed the inert
  `loop y : partition=rows;` line; header comment in the .sched.nuc
  captures rationale + cites TASK-0249 + names TASK-0250 follow-up. Cell
  is SKIPPED across all 4 tier-1 backends (TASK-0117 / TASK-0181 /
  TASK-0042.02), so bit-identical-preserving for every required cell.
- Caveat comment at `passes/partition_workers.rs:40` updated from
  "not yet filed" → "rejected at sched-lower as UnsupportedPartitionKind
  (TASK-0249)" — closes the doc-lie.
- Follow-up: TASK-0250 ("05-stencil/distributed: decide row-band
  partitioning of the y-loop") filed.

Tests:
- `negative_partition_rows_is_rejected`, `negative_partition_blocks2d_is_rejected`,
  `positive_partition_workers_still_lowers` in tests/sched_lower.rs.
- Loop-count expectations in `parses_05_stencil_distributed`,
  `lowers_05_stencil_distributed` updated from 2 → 1 (with comments).
- link test `links_05_stencil_distributed` unchanged (only asserts link
  succeeds; loop-count not asserted there).

Gates (run via `nix develop -c just <recipe>`):
- `just check`: clean.
- `just clippy`: clean (`-D warnings`, `--all-targets`).
- `just test`: full workspace tests pass (sched_lower 74/74 incl. 3 new).
- `just e2e`: total 88, pass 70, fail 0, skipped 18, required-fail 0.
  Matches the recorded baseline (memory: project-cross-backend-differential).
- `just determinism-check`: byte-identical across two builds (88 cells).
- `just determinism-check-negative`: correctly bit
  (NUC_NONDET_PERTURBED_CELLS=70).
- `just xbackend-check-negative`: correctly bit
  (NUC_XBACKEND_CORRUPTED_APPLIED=16, NUC_XBACKEND_CORRUPTED_DETECTED=1).

Honest limits / surprises:
- The migration removed the directive rather than rewriting it to
  `partition=workers`. The latter would change the IR + generated code
  shape and cannot be justified by this task's "close the silent-drop"
  scope; TASK-0250 captures the open question.
- All 4 tier-1 backends still SKIP 05-stencil/distributed for
  pre-existing reasons (TASK-0117 / TASK-0181 / mp-tcp-event Stage 3),
  so the cell's bit-identical status was not directly verifiable here.
  When any of those gaps closes and the cell becomes [[required]],
  TASK-0250's decision will determine whether the y-loop directive
  returns (as `partition=workers`) before bit-identical comparison.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Closes the silent-drop landmine on PartitionKind::Rows / Blocks2d
(both parsed and lowered but never consumed; PRD §6.3.3 forbids
silent-default acceptance). Implementation: typed reject at sched-
lower via the new SchedLowerErrorKind::UnsupportedPartitionKind
variant; the live 05-stencil/distributed schedule migrated by
removing the now-inert directive (the cell is SKIPPED across all
4 tier-1 backends for pre-existing gaps, so removal is bit-identical
for every required cell). passes/partition_workers.rs:40 caveat
comment updated from "not yet filed" to reflect the new
sched-lower rejection (closes the doc-lie). Follow-up TASK-0250
captures the open question of whether the y-loop should be
explicitly partitioned via `partition=workers` once the multi-
worker backend gaps close. Lands cycle 70.
<!-- SECTION:FINAL_SUMMARY:END -->

<!-- AC:END -->

<!-- AC:END -->
