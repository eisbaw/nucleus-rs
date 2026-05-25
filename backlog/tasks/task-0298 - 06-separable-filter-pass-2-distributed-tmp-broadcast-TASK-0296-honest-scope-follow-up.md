---
id: TASK-0298
title: >-
  06-separable-filter pass-2 distributed: tmp broadcast (TASK-0296 honest-scope
  follow-up)
status: To Do
assignee: []
created_date: '2026-05-25 01:12'
labels:
  - M5
  - compiler
  - partition
  - broadcast
  - 06-separable-filter
  - forward-carried-from-TASK-0296
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background
TASK-0296 cycle 116 landed the first M5 capstone cell for example 06: pass 1 (hblur_acc) distributed across {w0..w3} with partition=rows on hy; pass 2 (vblur_acc) stays on host. The honest-scope decision was driven by transfer_inject behaviour on N-to-M shapes (see transfer_inject.rs module docs §"N-to-M fan-out").

## What this task addresses
A both-passes-distributed variant: pass 2 also placed on {w0..w3} with partition=rows on vy. The complication is that vblur_acc reads `tmp[vm][vx]` where `vm` iterates `0..H` for every output (vy, vx) — each worker needs the FULL `tmp` matrix.

## Acceptance criteria
1. Investigate whether transfer_inject already handles broadcast (1-to-N) of `tmp` to every worker for pass 2 (each producer worker w_i has its hy row-band of tmp; each consumer worker w_j needs ALL of tmp). The "N-to-M fan-out" warning in transfer_inject.rs may or may not apply when the consumer needs the FULL data (no per-tile slicing on the receiver side); a 1-to-N broadcast where each producer is a different worker but each consumer needs the WHOLE data is conceptually N-to-1 (gather) on the producer side + 1-to-N (broadcast) on the consumer side, which composes into N-to-N but each pair is whole-array.
2. If broadcast works as-is: add nuc-nucleus/examples/06-separable-filter/schedules/distributed2.sched.nuc (or rename the current distributed.sched.nuc and rework — naming choice TBD by implementer) with both passes on { w0..w3 } and both `loop hy / loop vy : partition=rows`. Bit-identical against reference.bin.
3. If broadcast does NOT work as-is: file the gap as a precise transfer_inject task; this task stays In Progress as the smoke-test target for that fix.
4. Decide naming + placement: distributed2.sched.nuc vs replacing the current cell (the current "distributed" cell would become "distributed.pass1only" or similar). Document the rationale in the schedule comment.

## Honest scope
- LOW priority. The current TASK-0296 cell already exercises partition=rows on a non-stencil shape AND validates the cycle-116 mp-tcp-bufsync slice-paste fix across all 4 tier-1 backends. The both-passes variant is icing — proves the broadcast path, not a new M5 acceptance gap.
- Trigger: implementer with bandwidth for transfer_inject investigation.

## Forward-carry from TASK-0296 cycle 116
- mp-tcp-bufsync now uses the shared `backend_common::multi_worker_walker::render_wait_assign` for its Event::Wait emit (was: silent-sibling defect, whole-array overwrite). Any new Wait code path in mp-tcp-bufsync MUST go through this helper.
- The render_wait_assign signature was refactored cycle 116 to take `(sidecar, pair_tiles, ...)` instead of `(WalkerCtx, ...)` so backends that do not construct a full WalkerCtx (mp-tcp-bufsync) can call it directly.
<!-- SECTION:DESCRIPTION:END -->
