---
id: TASK-0327
title: >-
  mp-tcp worker-to-worker mesh / host-relay codegen for shared-set
  partitioned-data fan-out (TASK-0324 AC#3 cycle-147 follow-up)
status: To Do
assignee: []
created_date: '2026-05-25 16:04'
labels:
  - M6
  - backend
  - mp-tcp-bufsync
  - mp-tcp-event
  - topology
  - forward-carried-from-TASK-0324
dependencies:
  - TASK-0324
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0324 AC#3 (cycle 147) landed cross-worker tmp codegen for the same-set producer-set==consumer-set + reader-iv-exceeds-producer-tile shape (06-separable-filter/distributed2 reproducer). Verified bit-identical on the two shared-memory backends (pthreads-sync, pthreads-async).

The two mp-tcp backends (mp-tcp-bufsync, mp-tcp-event) cannot lower the 12 cross-worker (src, dst) Push/Wait pairs that the pass now emits — their one-(data,ctrl)-pair-per-(host,worker) STAR topology has no worker-to-worker channel. EmitError::ContractGap fires LOUD at Plan::build with the verbatim messages:

- mp-tcp-bufsync: `mp-tcp-bufsync's one-(data,ctrl)-pair-per-(host,worker) topology has no worker-to-worker channel (filed as TASK-0175)`
- mp-tcp-event:    `the star topology requires host as the relay (filed as TASK-0175)`

TASK-0175 was closed cycle-77 as DEFERRED-until-TASK-0117-lands-AND-a-distributed-schedule-needs-worker-to-worker (AC#3 of TASK-0175). Both conditions are now met: TASK-0117 fan-out has been live for many cycles, and 06/distributed2 is the in-tree schedule that exercises the worker-to-worker shape.

## Acceptance criteria

### AC#1: mp-tcp-bufsync worker-to-worker channel

Extend mp-tcp-bufsync's transport so a Push from WorkerId(i) to WorkerId(j) (i, j both non-host) routes correctly. Two viable approaches:

- **Full mesh**: each worker opens a (data, ctrl) connection pair to every other worker at startup. N*(N-1) connections per worker = quadratic in worker count but eliminates the host as a bottleneck.
- **Host relay**: workers route worker-to-worker Push/Wait through the host; host forwards the payload. Lower connection count, host becomes hot-path bottleneck. Acceptable for the cycle-147 distributed2 shape since the 12 cross-pairs are amortised over the H*W vblur loop body.

The simpler near-term fix is host-relay; mesh is the M6+/M7 target.

### AC#2: mp-tcp-event worker-to-worker channel

Same shape as AC#1 for mp-tcp-event. The mio reactor + per-(seq, peer) outbound queue (TASK-0042.05 Stage 3) already provides the per-peer fan-out machinery; the gap is the connection topology (no worker-to-worker socket pair exists at startup).

### AC#3: 06-separable-filter/distributed2 promotion

Once AC#1 + AC#2 land, flip the two [[skip]] entries in nuc-nucleus/e2e-matrix.toml lines ~1290-1310 (TASK-0327-citing) to [[required]] and verify bit-identical against reference.bin. e2e baseline shifts by +2 [[required]] -2 [[skip]].

## Honest scope

- MEDIUM priority. The cycle-147 AC#3 codegen already produces correct output on the two shared-memory backends (50% of the tier-1 matrix); mp-tcp coverage is the M5/M6 cross-backend completeness story.
- Trigger: M6 acceptance criterion or a follow-up that needs the full tier-1 matrix bit-identical on 06/distributed2.

## Dependencies

- TASK-0324 (cycle 147 AC#3 landed): `producer-set == consumer-set` fan-out emission. This task lifts the resulting topology gap that surfaces on mp-tcp backends.
- TASK-0175 (Done, deferred-until): the original filing of the mp-tcp worker-to-worker limitation. Now actionable per its own reopen-criterion.

## Cross-reference

- nucleus/backends/mp-tcp-bufsync/src/lib.rs (the host-only EventList Plan::build branch).
- nucleus/backends/mp-tcp-event/src/lib.rs (the host-relay-requires Push branch).
- 06-separable-filter/distributed2 emits 12 cross-pairs (4*3 = 12) under pthreads-sync; same count expected for the eventual mp-tcp implementations.
<!-- SECTION:DESCRIPTION:END -->
