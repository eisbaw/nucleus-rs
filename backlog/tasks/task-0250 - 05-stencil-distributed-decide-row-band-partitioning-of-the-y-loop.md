---
id: TASK-0250
title: '05-stencil/distributed: decide row-band partitioning of the y-loop'
status: Done
assignee: []
created_date: '2026-05-23 14:45'
updated_date: '2026-05-23 18:26'
labels:
  - compiler
  - partition
  - M3
  - follow-up
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0249 removed the inert `loop y : partition=rows;` line from `nuc-nucleus/examples/05-stencil/schedules/distributed.sched.nuc` because the directive had no downstream consumer (silent no-op). The schedule still distributes work via `place blur3 on { w0, w1, w2, w3 }` plus the existing transfer directives — but the OUTER y-loop is no longer explicitly partitioned in any source-visible way.

## Question

Should the y-loop in `05-stencil/distributed` be partitioned across the four compute workers via `partition=workers`? Today the cell is SKIPPED across all four tier-1 backends (pthreads-sync, mp-tcp-bufsync, pthreads-async, mp-tcp-event):

- pthreads-sync / mp-tcp-bufsync: capability mismatch on `async + buffer=2 + notify=event` (TASK-0117 + halo).
- pthreads-async: TASK-0181 (multi-worker strip-mine rebinding gap).
- mp-tcp-event: multi-worker codegen incomplete (Stage 3 — TASK-0042.05).

So a change to the y-loop partition policy does NOT shift any required e2e cell today — but it would change the IR + generated code shape if and when the cell becomes required.

## Decision needed

(a) Add `loop y : partition=workers;` — closes the partition-policy gap with real semantics. Changes downstream IR (per-worker loop-bound rewrite via TASK-0212).
(b) Leave the y-loop unpartitioned at the schedule layer — the `place blur3 on {w0..w3}` directive plus per-worker transfers carry the multi-worker shape implicitly.
(c) Implement a real `partition=rows` consumer (sibling pass to partition_workers.rs) and restore the original directive.

This task captures the open question; it is NOT a blocker for TASK-0249.
<!-- SECTION:DESCRIPTION:END -->

<!-- AC:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 #1 Decision recorded.
- [ ] #2 #2 If (a) or (c), schedule change lands and any cell promotion / regression is reported with bit-identical evidence.
- [x] #3 #3 If (b), this task closes with the rationale captured in the schedule's header comment.
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Orchestrator-direct cycle (no implementer). Resolution: option (b) — leave the y-loop unpartitioned at the schedule layer. Rationale recorded in distributed.sched.nuc header comment (this cycle's edit): today's cell is SKIPPED across all 4 tier-1 backends (TASK-0117 + halo on pthreads-sync/mp-tcp-bufsync; pthreads-async via TASK-0181 now closed but capability mismatch still gates; mp-tcp-event Stage 3 deferred to TASK-0042.05). Switching to partition=workers would change IR + generated-code shape (per-worker loop-bound rewrite via TASK-0212) without unblocking any required cell, so it's a no-op risk today. When the cell becomes [[required]] on at least one backend, revisit with option (a) (promote to partition=workers) if the emitted shape needs the explicit y-band; the place blur3 on {w0..w3} directive + per-worker transfers carry the multi-worker shape implicitly in the meantime. AC#2 N/A (only fires under options (a)/(c)). Gate: zero behaviour change — comment-only edit; e2e/determinism/negative gates unchanged from TASK-0181 cycle 73 baseline.
<!-- SECTION:FINAL_SUMMARY:END -->
