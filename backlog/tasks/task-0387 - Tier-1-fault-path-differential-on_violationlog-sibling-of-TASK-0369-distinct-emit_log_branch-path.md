---
id: TASK-0387
title: >-
  Tier-1 fault-path differential: on_violation=log sibling of TASK-0369
  (distinct emit_log_branch path)
status: Done
assignee: []
created_date: '2026-05-31 06:10'
updated_date: '2026-05-31 06:17'
labels:
  - e2e
  - runtime-check
  - robustness
  - fault-assert
dependencies:
  - TASK-0369
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
SIBLING of TASK-0369 (which covered on_violation=count on tier-1, cycle-222). The `[[fault_assert]]` harness mechanism now exists; this exercises the DISTINCT log emit path. `check_frame.rs::emit_log_branch` is a different code path from the count path (emit_count_branch + emit_count_reporter_struct Drop) — count was covered by TASK-0369 but log was NOT, so emit_log_branch has zero tier-1 cross-backend coverage.\n\nPlan: add 01-elementwise-add/schedules/check_log.sched.nuc (naive + `check loop i : latency_max = 1ns, on_violation = log`). The log branch prints PER-violation inline to stderr: `warning: check loop `i` violated latency_max=1 ns: iteration took {N} ns` where {N}=_check_elapsed (NON-deterministic). Pin ONLY the timing-INDEPENDENT prefix `warning: check loop `i` violated latency_max=1 ns: iteration took ` via [[fault_assert]] stderr_contains — NOT the ns value (same AC#3 honest-scoping as count). Empirically sweep all 7 tier-1 backends (build+run+check prefix-present+output.bin bit-identical), promote [[required]]+[[fault_assert]] on those that surface it; honest [[skip]] otherwise. Verify determinism (the schedule emit must be byte-identical). EXPECTED: like count, all 7 surface it via the single-worker shared renderer. Bounded: no core codegen changes, reuses the TASK-0369 mechanism.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
LANDED cycle-222 (commit 21de32a). Empirical 7-backend sweep: every tier-1 backend builds + runs rc=0, output.bin BIT-IDENTICAL to reference.bin, AND emits 256 log lines whose timing-INDEPENDENT prefix `warning: check loop `i` violated latency_max=1 ns: iteration took ` is PRESENT. All 7 promoted [[required]]+[[fault_assert]] (over the >=N-backend floor — same single-worker shared-renderer delegation as count). emit_log_branch lives in pthreads-sync/src/lib.rs (the shared single-worker renderer all backends delegate to for host-only schedules), which is why a helper that greps to ONE backend's src still emits on all 7.

HONEST SCOPE (AC#3): the pinned substring ends at 'iteration took ' and EXCLUDES the per-line elapsed `<N> ns` (timing-derived). The `1 ns` inside the prefix is the fixed schedule THRESHOLD (latency_max=1ns), not elapsed. The log path is LESS pinnable than count (one differing-ns line per violation vs count's single Drop summary) — only the prefix is deterministic.

REVIEW: orchestrator-inline self-review (matrix + fixture only, NO Rust source changed; the [[fault_assert]] MECHANISM this reuses got an independent mped-architect GO last cycle on TASK-0369). Verified: matrix has 7 distinct backends for both required+fault_assert (no dup/missing); the shipped-manifest no-orphans canary passes; determinism 7/7; full e2e 350/293/0/57/0. Bite is the same missing_fault_substring code proven end-to-end for count (FAIL/fault, required-fail=1).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
TASK-0387 DONE (cycle-222, commit 21de32a). Closed the on_violation=log tier-1 coverage gap — the DISTINCT emit path (check_frame.rs::emit_log_branch, the inline per-iteration warning) that TASK-0369's count cells did not exercise.

DELIVERED: nuc-nucleus/examples/01-elementwise-add/schedules/check_log.sched.nuc (naive + `check loop i : latency_max=1ns, on_violation=log`) + 7 [[required]] + 7 [[fault_assert]] matrix entries for check_log × all 7 tier-1 backends (M6). NO harness code change — reuses the TASK-0369 [[fault_assert]] mechanism.

HONEST SCOPE: the log branch prints ONE stderr line PER violation (256 here): `warning: check loop `i` violated latency_max=1 ns: iteration took <N> ns`. The fault_assert pins ONLY the timing-INDEPENDENT line PREFIX `... iteration took ` (excludes the timing-derived <N> ns); the `1 ns` in the prefix is the fixed threshold, not elapsed.

GATE: empirical 7-backend sweep all rc=0 / output.bin BIT-IDENTICAL / prefix PRESENT (256 lines); just e2e 343/286/0/57/0 -> 350/293/0/57/0 (+7, 0 fail, 0 required-fail); determinism (scoped) 7/7 byte-identical; shipped-manifest fault_assert canary + narrative-doc-lie + doc-citation(x2) fences OK. No Rust source changed, so clippy/test/test-release from TASK-0369's commit (1cc1684) still hold.

REVIEW: orchestrator-inline (the reused [[fault_assert]] mechanism was independently mped-architect-GO'd last cycle on TASK-0369; this is a matrix+fixture-only sibling — proportional review per the batch-qa-gate guidance).
<!-- SECTION:FINAL_SUMMARY:END -->
