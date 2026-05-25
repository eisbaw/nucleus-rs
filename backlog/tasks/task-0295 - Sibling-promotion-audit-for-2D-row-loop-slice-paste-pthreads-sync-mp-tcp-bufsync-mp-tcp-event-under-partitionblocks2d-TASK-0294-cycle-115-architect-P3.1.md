---
id: TASK-0295
title: >-
  Sibling-promotion audit for 2D row-loop slice-paste: pthreads-sync /
  mp-tcp-bufsync / mp-tcp-event under partition=blocks2d (TASK-0294 cycle-115
  architect P3.1)
status: In Progress
assignee:
  - '@mark'
created_date: '2026-05-25 00:27'
updated_date: '2026-05-25 11:52'
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 139 AC#3 audit (orchestrator-direct)

### Audit scope

Scanned `nucleus/backend-common/tests/wait_assign_slice.rs` and all callers of `render_wait_assign` / `render_worker_events` for tests that pin the multi_worker_walker 2D-tile emit string across the four tier-1 backends.

### Findings

- The four tier-1 backends use four distinct `rendezvous_prefix` values:
  - pthreads-sync: `"slot"` (backends/pthreads-sync/src/multi_worker.rs:536)
  - pthreads-async: `"ring"` (backends/pthreads-async/src/multi_worker.rs:516)
  - mp-tcp-event: `"chan"` (backends/mp-tcp-event/src/multi_worker.rs:493)
  - mp-tcp-bufsync: bypasses `render_worker_events`; calls `render_wait_assign` directly (backends/mp-tcp-bufsync/src/lib.rs:1196), no prefix involved.

- All existing 2D-tile pins in `wait_assign_slice.rs` (`rows_2d_slice_paste_for_partition_blocks2d`, `task0316_inner_axis_leading_layout_emits_against_dim0`, `task0316_non_prefix_layout_empty_bounds_consumer_pin`) feed `rendezvous_prefix: "ring"` via the `render_one_wait` helper. No 2D test exercises a non-`"ring"` prefix.

- Prefix substitution machinery is rendezvous_prefix-agnostic at multi_worker_walker.rs:809 — a single `format!("{prefix}{rendezvous_prefix}_{rid}.wait()")` with no prefix-conditional branches in the 2D row-loop dispatch. So substantively the dispatch IS shared correctly across all 4 backends; the test gap is in the test surface, not the production code.

### Outcome

Filed gap as **TASK-0321** (wait_assign_slice: parametric 2D-tile pin across all 4 rendezvous_prefix values). LOW priority — defensive coverage, no current defect. The gap would bite if a future refactor hardcoded `"ring_"` inside the 2D arm; the parameterised test would catch it.

### AC status (cycle 139)

- **AC#1**: trigger NOT MET. Requires "new partition=blocks2d schedule that is capability-compatible with pthreads-sync OR mp-tcp-bufsync" (i.e., not async/buffer/event). Today no such schedule exists. Remains conditional on the trigger materialising.
- **AC#2**: trigger NOT MET. Requires TASK-0175 (w↔w mesh on mp-tcp-event) to land. Today blocked. Remains conditional on TASK-0175.
- **AC#3**: **DONE** — audit complete, gap filed as TASK-0321.

### Honest status

AC#3 is closed. AC#1 + AC#2 are unconditionally trigger-gated and cannot be satisfied today. Per honest-failure discipline, this task stays In Progress (not Done) until the triggers materialise. The notes record the closure of AC#3 so a future cycle picking this up cold sees what's actually remaining.
<!-- SECTION:NOTES:END -->
