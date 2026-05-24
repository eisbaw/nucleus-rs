---
id: TASK-0267
title: >-
  transfer_inject host-side Push synthesis drops for partitioned consumers under
  inner block transform OR async transfer
status: To Do
assignee: []
created_date: '2026-05-24 08:02'
labels:
  - M5
  - bug
  - compiler
  - transfer_inject
dependencies:
  - TASK-0266
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

Spun out of TASK-0266 cycle-85 precise diagnosis (2026-05-24). When 05-stencil/distributed × pthreads-async is promoted from [[skip]] to [[required]], the generated binary hangs because the host fn main contains zero ring_*.push(img_in) calls. Workers w0..w3 block forever on w_i_ring_i.wait().

## Root cause (verified by Petri-net diff across partition policies)

Same algorithm + same place + same workers, ONLY the partition policy differs:
- partition=workers (WORKS): Petri net contains push_seq0..7 — 4 host-side Pushes for img_in + 4 worker Pushes for img_out. Host emit shows ring_0..3.push(img_in.clone()) at top level.
- partition=rows (BROKEN): Petri net contains push_seq4..7 ONLY — only the worker-side Pushes for img_out exist. The host-side Pushes for img_in (push_seq0..3) are ENTIRELY MISSING.

Additionally, cycle-85 ATTEMPT-2 showed that even with partition=workers (the working policy) the Push synthesis ALSO drops when EITHER of these is present:
- inner block transform on the partitioned axis: `loop x : block=64, vectorize=8, reuse;`
- async transfer mode: `transfer img_in : async, buffer=2, notify=event;`

Removing both restored host-side Push synthesis under partition=workers. Under partition=rows the host-side Push drops unconditionally.

## Probable bug site

transfer_inject (passes/transfer_inject.rs) — one of the following passes drops the host-side Push for partitioned consumers when (partition_rows) OR (partition_workers + block transform) OR (partition_workers + async transfer):

1. hoist_invariant_waits — for a 2D nest with partition on the OUTER axis, the inner (x) loop may not be transparent to invariance checking. Pass A may not recognise img_in as invariant w.r.t. the partition axis when the inner loop is a block-transformed strip.
2. splice_pushes_global / splice_pushes_for_waits — the cut decision (lowest common Repeat enclosing producer but not wait) may fail to place a Push at the correct level when the consumer Waits are nested inside a partition Repeat AND a block Repeat.
3. canonical-worker collapse + fan-out expansion (pre-TASK-0117) — the host-Push site may key off a check that recognises partition=workers but not partition=rows, OR that fails when the consumer side has a block Repeat in the path.

## Reproduction

- Schedule: nucleus/compiler/tests/fixtures/examples/05-stencil/distributed.sched.nuc
- partition=rows × any inner block × any transfer mode: deadlock, no host-side Push in emit.
- Swap to partition=workers + remove inner block + sync transfer: WORKS bit-identical.
- Swap to partition=workers + KEEP inner block OR async transfer: deadlock, no host-side Push.

## Probe recipe (next implementer)

Add eprintln! instrumentation to transfer_inject.rs at:
- build_waits_for_op exit (line ~2089): print the Vec<XferPlaceholder> waits emitted.
- splice_pushes_for_waits exit (line ~963): print inserts performed.
- splice_pushes_global enter+exit (lines ~1461 / ~1591): print waits collected and pushes spliced.
- inject_transfers exit (line ~411): print final ACFG node count + Xfer count by role + Xfer roles by src/dst.

Run with partition=rows AND partition=workers + with/without inner block + sync/async transfer. The ONE pass whose output diverges is the bug site.

## Acceptance criteria

1. Failing fixture in nucleus/compiler/tests/ that pins the bug as a typed transfer_inject ContractGap (or a positive test asserting host-side Push presence in the lowered ACFG for partitioned consumers + inner block + async transfer combos).
2. Fix lands so the Petri net for 05-stencil/distributed × pthreads-async (with partition=workers AND inner block AND async transfer) contains the host-side Push events.
3. e2e cell 05-stencil/distributed × pthreads-async promotes from [[skip]] to [[required]] AND PASSES bit-identical to reference.bin UNDER partition=workers + inner block + async transfer.
4. partition=rows variant ALSO passes (closes the second arm of the bug; if partition=rows-specific extension is non-trivial beyond the block/async fix, allowed to defer to TASK-0258.bis with a precise note here).
5. Regression test: a multi-policy + multi-block + multi-transfer-mode fixture so a future regression in any direction surfaces.

## Honest limits / scope

- DO NOT fix BUG 2 (per-iteration barrier deadlock) in this task — it's separate (TASK-0268). The two interact: even after this fix, partition_rows × unequal-iter still deadlocks. The two must BOTH land for M5 AC#4 to close.
- Allowed scope: transfer_inject (passes/transfer_inject.rs) + closely-related sidecar/ACFG plumbing. NOT sync_inject; NOT backend codegen.

## Dependencies

- Prerequisite of: TASK-0266 (M5 AC#4 closure umbrella).
- Sibling: TASK-0268 (BUG 2 barrier-deadlock).

## Forward-carry context

Memory entries that bear:
- project_partition-silent-drop (partition_rows = TASK-0258 cycle 79c; transfer_inject was never updated for partition_rows consumers).
- project_negative-seam-and-backend-layout (transfer_inject pass landscape, post-fan-out shape).
<!-- SECTION:DESCRIPTION:END -->
