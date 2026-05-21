---
id: TASK-0215
title: Document and test block=N + pipeline=D combination semantics on the same loop
status: Done
assignee: []
created_date: '2026-05-21 14:10'
updated_date: '2026-05-21 19:40'
labels:
  - compiler
  - docs
  - M4
dependencies:
  - TASK-0134
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architecture-review finding (TASK-0134 cycle): when both block= and pipeline= apply to the same loop variable, block_transform tiles the loop into outer/inner; the IterVar id is reused for the inner intra-tile loop (block_transform.rs); pipeline=D therefore applies to the INNER loop. No test exercises this combination; no code-level docstring captures it; users could be surprised by the per-tile (not per-iteration) pipeline-depth semantic.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Either: (a) add a documented + tested semantic (block=N + pipeline=D means D in-flight intra-tile firings per tile; with N>=D the buffer place still gets initial_marking=D); OR (b) reject the combination with a typed error at SchedLowerError or LinkError.
- [ ] #2 Add a synthetic-ACFG unit test that constructs a Repeat-with-block + pipeline= scenario and asserts the chosen semantic; update block_transform.rs module doc with the chosen path.
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Done — orchestrator-direct cycle (resolution: PATH B reject)

AC#1 resolved with PATH B (reject the combination at sched-lower). Rationale: PRD §6.3.3 explicitly says "Loop options are orthogonal where possible. Bad combinations ... are rejected at compile time, not at runtime." The block + pipeline combo on the same loop has ambiguous semantics (per-tile vs per-iter pipelining); block_transform's current iter-var reuse would silently land pipeline=D on intra-tile iterations, almost certainly not the schedule author's intent. Reject is the honest answer; pick ONE of {block, pipeline} per loop.

PATH A (document + test per-tile semantic) was rejected: there is no canonical "what should this mean" that aligns with PRD framing; codifying a guess locks in a possible footgun.

### Implementation
- `nucleus/compiler/src/sched/ir.rs`: new `SchedLowerErrorKind::BlockPipelineConflict { var }` variant + Display message naming PRD §6.3.3 + the two actionable fixes (drop block OR drop pipeline).
- `nucleus/compiler/src/sched/lower.rs`: post-options-lowering gate in `lower_loop` — if both Block and Pipeline are present, emit the variant at the loop's var_span. Cascade-class table updated.
- `nucleus/compiler/src/passes/block_transform.rs` module doc: line about block+pipeline updated from "silent under-tested area" to "REJECTED at sched-lower (TASK-0215 — closed)".

### Tests (sched_lower.rs +1)
- `negative_block_pipeline_combination_on_same_loop_is_rejected`: `loop n : block=4, pipeline=2;` must fail with `BlockPipelineConflict { var: "n" }`.

### Gate (orchestrator re-ran)
- cargo test workspace: 548 pass / 0 fail / 2 ignored (was 547/0/2; +1).
- cargo clippy --workspace --all-targets -- -D warnings: clean.
- just e2e: 36 cells: 29 / 0 / 7 (baseline unchanged).
<!-- SECTION:NOTES:END -->
