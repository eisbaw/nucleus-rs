---
id: TASK-0214
title: >-
  Same-worker transfer DATA : buffer=N directive should either be rejected or be
  carved out of PipelineExceedsBuffer check
status: Done
assignee: []
created_date: '2026-05-21 14:10'
updated_date: '2026-05-21 19:11'
labels:
  - compiler
  - link
  - M4
  - latent
dependencies:
  - TASK-0134
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Architecture-review finding (TASK-0134 cycle): the link-step PipelineExceedsBuffer check fires whenever a data symbol with a transfer directive has D > N and both producer/consumer kernels are inside the pipelined loop. The IR (transfer_inject) does NOT emit an Xfer for same-worker producer/consumer (src==dst at transfer_inject.rs:1717). So a pathological schedule with transfer X : buffer=1 on a same-worker symbol + pipeline=3 would emit PipelineExceedsBuffer despite no actual constraint existing in the lowered IR. Latent inconsistency. Currently the link.rs:669-685 doc-comment acknowledges this and points at this task; the in-tree examples don't hit it.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Decide one path: (a) reject 'transfer X : buffer=N' when src==dst is structurally inevitable from the schedule, with a SchedLowerError naming X and the placement that makes it same-worker; OR (b) gate check_pipeline_buffer_constraints on the kernel placements (skip when producer/consumer share a worker).
- [ ] #2 Test: positive — same-worker producer/consumer with redundant transfer directive AND pipeline=2 + buffer=1 must either fail at SchedLower (path a) or link cleanly (path b). Document the choice in link.rs:669-685 with the new ground truth (replacing the current 'Caveat TASK-0214' note).
- [ ] #3 Forward-carry into TASK-0042.01: if path (a) is chosen, pthreads-async codegen never sees same-worker transfer directives; if path (b), it must skip same-worker transfers in its ring-buffer setup.
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Done — orchestrator-direct cycle (resolution: PATH B)

AC#1 resolved with PATH B (gate check_pipeline_buffer_constraints on cross-worker placement). Rationale: PATH A (reject same-worker transfer directives at sched-lower) would break existing examples that have transfer directives on data symbols whose actual placement turns out to be same-worker — harmless today but flagged as defects. PATH B keeps current acceptance semantics + closes the inconsistency the architect flagged.

### Implementation
- `nucleus/compiler/src/link.rs`:
  - `data_is_cross_worker(algo, sched, data_name) -> bool`: walks `algo.stmts`, collects kernels touching `data_name`, looks up `sched.places[kernel].target` (handles both `One(w)` and `Many(ws)` variants), returns true iff MORE THAN ONE distinct worker is named.
  - `collect_kernels_touching_data` recursive helper handles Dataflow LHS+RHS (kernel = top-level Call callee), Effect args (kernel = callee), nested For-bodies.
  - `expr_touches_data` traverses BinOp/Neg/Call/DataRef.
  - Unplaced kernel conservatively returns true (don't squelch the check on a broken schedule).
  - `check_pipeline_buffer_constraints` gated with `if !data_is_cross_worker(algo, sched, data_name) { continue; }`.
  - Caveat docstring (lines ~669-685) rewritten from "TASK-0214 latent inconsistency" to "TASK-0214 — closed".

### Tests (link.rs +2)
- `pipeline_buffer_check_skips_same_worker_data`: both f1+f2 on w0; pipeline=4 vs buffer=1 on `stage1` — must link cleanly (no error).
- `pipeline_buffer_check_still_fires_on_cross_worker_data`: f1=w0, f2=w1; same pipeline=4 vs buffer=1 — `PipelineExceedsBuffer` must fire.

### Gate (orchestrator re-ran)
- cargo test workspace: 547 pass / 0 fail / 2 ignored (was 545/0/2; +2).
- cargo clippy --workspace --all-targets -- -D warnings: clean.
- just e2e: 36 cells: 29 / 0 / 7 (baseline unchanged).

### Forward-carry
Appended to TASK-0042.01 (pthreads-async): same-worker data symbols produce no Xfer (TASK-0124 contract unchanged); no new ring-buffer code path needed for them.
<!-- SECTION:NOTES:END -->
