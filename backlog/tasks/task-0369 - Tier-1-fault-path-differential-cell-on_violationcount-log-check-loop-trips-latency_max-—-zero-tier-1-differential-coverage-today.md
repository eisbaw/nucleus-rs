---
id: TASK-0369
title: >-
  Tier-1 fault-path differential cell: on_violation=count/log check-loop trips
  latency_max — zero tier-1 differential coverage today
status: To Do
assignee: []
created_date: '2026-05-30 11:08'
updated_date: '2026-05-30 18:32'
labels:
  - e2e
  - runtime-check
  - robustness
  - M?
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Cycle-213 strategic-analysis finding (R3, robustness). VERIFIED: on_violation (count/log/panic) appears ONLY in embedded schedules (01-elementwise-add/embedded_check*, 14-hearing-aid/embedded_multimcu*), all req=0 in the tier-1 e2e matrix (validated solely by separate Renode recipes). So the entire runtime-assertion + fault-reporting surface is NEVER cross-backend differential-tested on tier-1 — the bit-identity invariant has zero coverage for the fault path. The inject_check_frames machinery already exists for tier-1 (TASK-0052). Add a tier-1 check-loop schedule that deliberately trips latency_max with on_violation=count (and/or log), producing a deterministic fault-report artifact, and promote it across the tier-1 backends.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A tier-1 (non-embedded) schedule exists with a check loop V : latency_max=T, on_violation=count (and/or log) where T is set so the violation deterministically FIRES
- [ ] #2 The fault-report output (the count/log artifact) is bit-identical across the tier-1 backends where the schedule is capability-compatible, and the cell is promoted [[required]] on those backends
- [ ] #3 Honest scoping: if on_violation=count/log output is inherently non-deterministic across backends (e.g. timing-derived), document precisely what IS pinned (e.g. the violation-count, not the latency value) and pin only that
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
FORWARD-CARRIED feasibility finding (cycle-214 orchestrator triage, NOT yet implemented). The fault-report artifacts are NON-trivial to differential-test, and AC#2 (bit-identical across backends) collides with the existing design. Evidence from backend-common/src/check_frame.rs: (1) BOTH the log branch (emit_log_branch :180) and the count Drop summary (emit_count_reporter_struct :106) write to STDERR, not stdout — the docstrings at :170-173 and :104-105 state this is DELIBERATE so the output.bin cross-backend differential stays stable/indifferent to check-loop presence. (2) The e2e harness compares output.bin only; it does NOT capture/compare stderr today. (3) The log message embeds _check_elapsed (actual wall-clock ns from monotonic_ns) -> inherently NON-deterministic across runs/backends. (4) count at latency_max=1ns is timing-dependent too: the compare is _check_elapsed > Nns, so an iteration measuring 0ns under coarse host clock resolution would NOT be counted -> count is not GUARANTEED bit-identical (this is exactly the 255-vs-256 band documented for the Renode embedded count fixture, but on tier-1 the work-per-iter makes >1ns near-certain, NOT certain). CONSEQUENCE: a standard output.bin differential cell would pass TRIVIALLY (fault goes to stderr, output.bin unchanged) WITHOUT testing the fault artifact at all. To honor AC#2 the harness must be extended to capture+compare stderr, and even then only a TIMING-INDEPENDENT invariant can be pinned (per AC#3): e.g. the count-summary LINE PRESENCE + loop-var name + threshold-ns echo, NOT the occurrence count or the elapsed ns. RECOMMENDED scoping for whoever picks this up: either (a) add a tier-1 e2e stderr-capture comparison mode + pin only the deterministic substring (presence + loop_var + threshold), explicitly NOT the count/ns; or (b) re-scope AC#2 to a single-backend golden-file pin of the fault line shape and document that cross-backend bit-identity of the count is not robustly achievable. This is the kind of design decision that belongs in the brief, not discovered mid-implementation.
<!-- SECTION:NOTES:END -->
