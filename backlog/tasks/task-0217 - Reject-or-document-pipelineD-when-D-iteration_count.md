---
id: TASK-0217
title: Reject or document pipeline=D when D > iteration_count
status: Done
assignee: []
created_date: '2026-05-21 14:36'
updated_date: '2026-05-21 18:44'
labels:
  - compiler
  - ir
dependencies:
  - TASK-0213
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
TASK-0213 (path 2 — push elision in acfg_to_petri) introduces a corner case: when pipeline=D > N (loop iteration count), the elision logic elides only N pushes (all of them), and the buffer place ends with D-N leftover tokens. Boundedness/deadlock passes still accept, but the analysis-net's end-state is non-empty for a finite loop — semantically odd.

Today's link step (TASK-0134 AC#3) only rejects D > buffer=N (where N is capacity), not D > iteration_count. They are different N's:
- D <= buffer=N: bounds the runtime ring-buffer.
- D <= iteration_count: ensures pipelining makes sense (you can't pipeline 2 iterations through 3 stages).

Acceptance criteria:
- #1 Decide: hard-reject D > iteration_count at link-time, OR document the analysis-net leftover-tokens as intentional under D > N.
- #2 If reject: extend check_pipeline_buffer_constraints (link.rs) with the new check, plus a precise LinkError variant + test.
- #3 If document: add a fixture covering D > N and update the acfg_to_petri module doc's elision section.

Discovered while implementing TASK-0213; out of scope for that task.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Done — orchestrator-direct cycle (resolution: reject at link time)

AC#1 resolved with PATH A (hard-reject at link-time, not document-as-intentional). Rationale: PRD §6.3.5 framing of pipeline=D as "head-start" assumes the head-start can drain through subsequent iterations; D > iter_count means the head-start never drains. Reject keeps the diagnostic at the user-source layer instead of surfacing as an analysis-net oddity later.

### Implementation
- `nucleus/compiler/src/acfg.rs`: `eval_const` made `pub(crate)` so link.rs reuses the same const-evaluator (no duplicate).
- `nucleus/compiler/src/link.rs`:
  - New `LinkError::PipelineExceedsIterationCount { loop_var, depth, iteration_count }` variant.
  - Display impl with diagnostic message naming all three numbers + reference to TASK-0217.
  - `check_pipeline_buffer_constraints` extended: after `let depth = ...`, if `find_loop_iter_count(stmts, var, consts)` returns `Some(n)` and `depth as i64 > n`, append the new error. Does NOT continue — if both D > iter_count AND D > buffer for some data, BOTH are reported (independent actionable fixes).
  - New helper `find_loop_iter_count` recursively walks algo.stmts looking for the matching For-loop and evaluates `hi - lo` via the (now pub(crate)) `eval_const`.
- `nucleus/compiler/src/passes/acfg_to_petri.rs`: removed the TASK-0217 honest-limitation bullet from the module doc (now resolved); annotated the elision call site to note "TASK-0217 closed — link step rejects D > iter_count BEFORE this pass runs".
- `nucleus/compiler/tests/link.rs`: 3 new tests
  - negative_pipeline_depth_exceeds_iteration_count: D=3 on 2-iter loop → exact LinkError variant with (n, 3, 2).
  - pipeline_iter_count_check_message_names_loop_and_numbers: message contains loop_var + depth + iter_count.
  - positive_pipeline_depth_equals_iteration_count: D=iter_count=2 with buffer=2 is the boundary case; links cleanly.

### Gate (orchestrator re-ran)
- cargo test workspace: 545 pass / 0 fail / 2 ignored (was 542/0/2; +3 new tests).
- cargo clippy --workspace --all-targets -- -D warnings: clean.
- just e2e: 36 cells: 29 / 0 / 7 (baseline unchanged).
<!-- SECTION:NOTES:END -->
