---
id: TASK-0211
title: >-
  Multi-worker transfer-distribution under partition=workers omits per-worker
  recv of partial-sub-array slot
status: To Do
assignee: []
created_date: '2026-05-20 21:43'
updated_date: '2026-05-20 21:43'
labels:
  - backend
  - codegen
  - M3
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Both tier-1 backends (pthreads-sync, mp-tcp-bufsync) implement the partition=workers loop-transform by replicating the loop body into per-worker scopes (w0/w1/w2/w3). When the body reads a partial sub-slice of a slot produced upstream (e.g. CNN's `input[n]` with `input: i32[B][C0][H][W]`), only the FIRST worker (w0) is emitted a `slot_X.wait()` to receive the data - w1/w2/w3 reference an undeclared local of the same name and the generated crate fails cargo build with E0425 "cannot find value `input` in this scope".

Reproducer (verified at TASK-0053 cycle-2):
  nuc-nucleus/examples/13-cnn-inference/schedules/batch_parallel.sched.nuc
  + pthreads-sync. `partition=workers` over the batch loop on workers
  {w0, w1, w2, w3}; each worker reads `input[n]` for its slice of n.

Pre-existing bug exposed by TASK-0209's partial-sub-array codegen path.
Examples 01..07's current required cells do not trigger it because none
combines partition=workers + multi-worker recv + sub-array read in one
scope (scalar-only partition=workers writes/reads happen to compile).

Expected behaviour (per the algorithm/schedule contract, PRD §10.1):
`transfer input : sync` plus partition=workers should emit, in each
compute worker's scope, EITHER (a) a recv of the worker's own slice of
`input` from host (preferred - minimal transfer), OR (b) a recv of
the whole `input` (current w0 emission) into a per-worker local. Both
spellings give a successful cargo build. Whichever the codegen picks
must be uniform across all participating workers.

Out of scope: the slice-of-batch optimisation is nice-to-have; the
correctness bar is that every per-worker scope binds `input` (and any
other shared slot read by its body) before referencing it.

Verification:
- Re-emit 13-cnn-inference batch_parallel x pthreads-sync; the
  generated crate cargo-builds.
- A NEW unit test in pthreads-sync renders a synthetic partition=workers
  loop whose body partial-indexes a shared slot, and asserts every
  worker scope receives a `slot_X.wait()` (or per-slice send) before
  the body references the slot.
- Promote example 13 batch_parallel x pthreads-sync from `[[skip]]`
  in nuc-nucleus/e2e-matrix.toml to `[[required]]`.

Out of scope (separate tasks):
- The mp-tcp-bufsync sibling cell also hits TASK-0175 (host-excluding
  barrier); fixing TASK-0211 alone does not unblock that cell.
- pipeline_parallel: TASK-0210 capability gap.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Multi-worker codegen on both tier-1 backends emits a uniform per-worker recv (or per-slice send) for every shared slot read by a partition=workers loop body, before the body's first reference to that slot.
- [ ] #2 Generated nuc-generated crate for 13-cnn-inference/batch_parallel/pthreads-sync cargo-builds without E0425.
- [ ] #3 New synthetic unit test in pthreads-sync renders a partition=workers loop with a partial sub-array body read and asserts every worker scope has the recv before the body reference.
- [ ] #4 13-cnn-inference batch_parallel × pthreads-sync moved from [[skip]] to [[required]] in nuc-nucleus/e2e-matrix.toml; cell is byte-identical to reference.bin.
- [ ] #5 01..07 cells unchanged (no regression of scalar-only partition=workers codegen).
<!-- AC:END -->
