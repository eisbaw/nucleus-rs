---
id: TASK-0295
title: >-
  Sibling-promotion audit for 2D row-loop slice-paste: pthreads-sync /
  mp-tcp-bufsync / mp-tcp-event under partition=blocks2d (TASK-0294 cycle-115
  architect P3.1)
status: To Do
assignee:
  - '@mark'
created_date: '2026-05-25 00:27'
labels:
  - M5
  - compiler
  - backend-common
  - partition
  - blocks2d
  - silent-sibling
  - forward-carried-from-TASK-0294
dependencies: []
priority: low
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0294 cycle 115 extends backend-common's wait_slice / render_wait_assign to handle 2D partition=blocks2d tiles via a new WaitSlice::Rows variant. Because backend-common is shared across all 4 tier-1 backends (pthreads-sync, pthreads-async, mp-tcp-bufsync, mp-tcp-event — TASK-0244 cycle 37), the 2D row-loop slice-paste fires automatically on any backend whose multi_worker_walker.render_worker_events is called with a 2-bound IterTile and 2-dim data.

## Today

Cycle 115 unblocks 05-stencil/distributed-2d × pthreads-async ([[required]] in nuc-nucleus/e2e-matrix.toml). The other three backends remain [[skip]] on that schedule for non-cycle-115 reasons:

- pthreads-sync + mp-tcp-bufsync: capability mismatch on `transfer img_in : async, buffer=2, notify=event` (TASK-0042).
- mp-tcp-event: w↔w mesh required for the 2x2-grid halo strips between (row, 0) and (row, 1) workers (TASK-0175).

## Why this matters

When any of those skips unlock (a new partition=blocks2d schedule WITHOUT the async/buffer/event capability requirement; or TASK-0175 lands worker-to-worker mesh), the 2D row-loop slice-paste will inherit silently into those backends. The inheritance is SOUND (the shared walker is the single source of truth), but it would be useful to file an EXPLICIT promotion task so the inheritance is intentional rather than accidental — guards against the feedback-silent-sibling-defect pattern in MEMORY.md.

## Acceptance criteria

1. When a new partition=blocks2d schedule lands that is capability-compatible with pthreads-sync OR mp-tcp-bufsync, file a positive-promotion task: add an e2e cell that exercises that backend's render_wait_assign on a 2D tile, bit-identical against a known reference. Aim for one cell per backend, parallel to the cycle-115 pthreads-async cell.
2. When TASK-0175 lands w↔w mesh on mp-tcp-event, promote 05-stencil/distributed-2d × mp-tcp-event from [[skip]] to [[required]] (the SKIP reason in nuc-nucleus/e2e-matrix.toml already explicitly cites both TASK-0175 AND TASK-0294 — cycle 115 closes the latter half).
3. Audit: scan for any test that pins the multi_worker_walker emit string on a 2D tile for a non-pthreads-async backend; if none exists, file the test gap as a follow-up.

## Cross-reference

- nucleus/backend-common/src/multi_worker_walker.rs (the shared writer — cycle 115 changes lines ~84-122 + ~840-1020).
- nucleus/backend-common/tests/wait_assign_slice.rs (the 7-test pin landed cycle 115; the test uses rendezvous_prefix="ring" for pthreads-async; the same code path serves pthreads-sync with prefix="slot").
- nuc-nucleus/e2e-matrix.toml lines 703-756 (the cycle-115 promoted cell + the 3 sibling SKIPs with their cited blockers).

## Honest scope

Speculative — files a future-promotion checklist, NOT new code. The shared walker means the BUG fix lands once; this task is the housekeeping to make sure the BENEFIT is realised on every backend when their upstream blockers clear. Low priority because the trigger (capability/mesh land) is gated on other tasks.
<!-- SECTION:DESCRIPTION:END -->
