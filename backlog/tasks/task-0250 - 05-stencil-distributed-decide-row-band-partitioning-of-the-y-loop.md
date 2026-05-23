---
id: TASK-0250
title: '05-stencil/distributed: decide row-band partitioning of the y-loop'
status: To Do
assignee: []
created_date: '2026-05-23 14:45'
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

Should the y-loop in `05-stencil/distributed` be partitioned across the four compute workers via `partition=workers`? Today the cell is SKIPPED across all three tier-1 backends:

- pthreads-sync / mp-tcp-bufsync: capability mismatch on `async + buffer=2 + notify=event` (TASK-0117 + halo).
- pthreads-async: TASK-0181 (multi-worker strip-mine rebinding gap).

So a change to the y-loop partition policy does NOT shift any required e2e cell — but it would change the IR + generated code shape if and when the cell becomes required.

## Decision needed

(a) Add `loop y : partition=workers;` — closes the partition-policy gap with real semantics. Changes downstream IR (per-worker loop-bound rewrite via TASK-0212).
(b) Leave the y-loop unpartitioned at the schedule layer — the `place blur3 on {w0..w3}` directive plus per-worker transfers carry the multi-worker shape implicitly.
(c) Implement a real `partition=rows` consumer (sibling pass to partition_workers.rs) and restore the original directive.

This task captures the open question; it is NOT a blocker for TASK-0249.

## Acceptance criteria

- [ ] #1 Decision recorded.
- [ ] #2 If (a) or (c), schedule change lands and any cell promotion / regression is reported with bit-identical evidence.
- [ ] #3 If (b), this task closes with the rationale captured in the schedule's header comment.
<!-- SECTION:DESCRIPTION:END -->
