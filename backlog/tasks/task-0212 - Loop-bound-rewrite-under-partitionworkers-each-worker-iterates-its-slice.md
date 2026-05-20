---
id: TASK-0212
title: 'Loop-bound rewrite under partition=workers: each worker iterates its slice'
status: To Do
assignee: []
created_date: '2026-05-20 22:07'
labels:
  - compiler
  - ir
  - partition
  - M3
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Surfaced during TASK-0211 cycle-2 STOP-AND-REPORT (mped-architect 2026-05-21).

The schedule directive `loop n : partition=workers;` on a loop whose
body is placed on a distributed entity (e.g. `place k on {w0,w1,w2,w3}`)
is currently a parse-and-lower-and-drop: parsed in
compiler/src/sched/parser.rs:575 as PartitionKind::Workers, lowered in
compiler/src/sched/lower.rs:874 to ResolvedLoopOption::Partition, then
NEVER consumed by any downstream pass. Only ResolvedLoopOption::Block
has a consumer today (compiler/src/passes/block_transform.rs:207).

Consequence (verified by inspecting the generated main.rs for
13-cnn-inference/batch_parallel/pthreads-sync): every compute worker
iterates the full source range (n in 0..16 for the CNN), redundantly
firing all kernels for the whole batch. The "each worker processes
B/N samples" semantic the batch_parallel schedule's comment promises
is not realised. The current output happens to be a correct full
result because (a) all four workers compute the same full result and
(b) the single Push back to host (canonical-first dst) happens to be
worker w0's full output. Both are fragile coincidences, not design.

Spec: a pass (peer to passes/block_transform.rs; could live as
passes/partition_workers.rs) walks the ACFG, finds ACFGNode::Repeat
nodes whose schedule directive carries
ResolvedLoopOption::Partition(PartitionKind::Workers) AND whose body
contains Operations placed on a multi-worker entity, then rewrites the
Repeat's range from a shared source range into a per-worker slice.
The natural implementation: have the projection
(petri_to_events::walk on Repeat) consume a per-worker range override
keyed by (WorkerId, IterVar), so each worker's Event::Loop carries its
own (lo, hi). NameSidecar::loop_bounds becomes per-worker for these
iter vars, or a sibling per-worker-loop-bounds map is added.

Honest scope:
- Divisibility: B/N exact split is simplest first cut. Non-divisible
  splits (e.g. 17 samples across 4 workers) need a tail-handling
  policy - file as a follow-up of this task, not as a blocker for
  the divisible first cut (16/4 in example 13, 4/4 in 03-reduction
  distributed).
- 1D partition axis only. partition=blocks2d / partition=rows are
  orthogonal grammar that need their own passes; this task is the
  partition=workers slice.
- Must compose with TASK-0117 (transfer-injection fan-out): once
  per-worker ranges land, the Push/Wait pairs need to carry the
  correct per-worker IterTile. The transfer_inject docs (lines
  78-84) already point at this future-partition pass.

Out of scope:
- TASK-0117 itself (transfer fan-out; that is the sibling task).
- mp-tcp-bufsync host-excluding barrier (TASK-0175).
- pipeline_parallel / async transfers (TASK-0210).

Acceptance criteria:
1. PartitionKind::Workers on a Repeat is consumed: each compute
   worker's Event::Loop carries its own per-worker range
   (range.start, range.end) such that the union of all workers'
   ranges covers the source range exactly once (B/N exact case).
2. petri_to_events emit walks the per-worker range when projecting
   the Repeat for that worker.
3. New unit test in nucleus/compiler/tests/ exercises a synthetic
   partition=workers loop with 4-element range across 2 workers and
   asserts each worker's projected Event::Loop has the correct
   (lo, hi).
4. No regression on 01..07 / 02-split / 03-reduction-naive cells
   (which use no partition=workers).
5. Composes with TASK-0117 so 13-cnn-inference/batch_parallel/
   pthreads-sync becomes byte-identical to reference.bin (closes
   TASK-0211 AC#4 jointly).
<!-- SECTION:DESCRIPTION:END -->
