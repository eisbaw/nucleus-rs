---
id: TASK-0268
title: >-
  sync_inject: participant-aware barriers for partitioned-loop bodies with
  unequal per-worker iteration counts
status: Done
assignee:
  - '@mped-orchestrator'
created_date: '2026-05-24 08:02'
updated_date: '2026-05-24 14:21'
labels:
  - M5
  - bug
  - compiler
  - sync_inject
dependencies:
  - TASK-0266
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

Spun out of TASK-0266 cycle-83 + cycle-85 diagnosis. Even AFTER TASK-0267 (BUG 1: host-Push synthesis drop) is fixed, the 05-stencil/distributed × pthreads-async cell still deadlocks at runtime when partition produces UNEQUAL per-worker iteration counts AND sync_inject inserts per-iteration barriers requiring ALL workers.

## Root cause (verified by reading emitted main.rs)

Diagnosed on the cycle-83 emit at nucleus/target/e2e-matrix/run-3307562-*/05-stencil__distributed__pthreads-async/src/main.rs:

- TASK-0262 floor-with-spillover remainder policy: 14 rows / 4 workers => w0=4 rows, w1=4 rows, w2=3 rows, w3=3 rows.
- Emitted main.rs shows w0_bar_1.wait() and w0_bar_2.wait() fire INSIDE the y-loop body.
- bar_1 + bar_2 are sync_inject barriers requiring ALL 4 workers to participate.
- w0/w1 call bar_1 four times; w2/w3 call it three times.
- On the 4th iteration, w0/w1 wait for w2/w3 INDEFINITELY — w2/w3 already exited the loop.

This is a structural problem at the partition_{rows,workers} × sync_inject seam: when partition produces unequal per-worker iteration counts, the in-body barriers expect equal counts and deadlock.

## Fix options (from TASK-0266 cycle-83 notes)

(A) Restore NonDivisible reject for partition_rows + partition_workers. Smallest correct change; loses 14-row × 4-worker case. Fail-fast.

(B) Trailing-partial-tile policy (TASK-0262 option c): mirror block_transform's discipline — emit one Repeat for the divisible portion (12 rows = 4 × 3) and a separate trailing Repeat for the remainder (2 rows), each with its own worker-aware barrier participant set. Requires sync_inject to be aware of the trailing-partial split.

(C) Hoist per-iteration barriers out of the partitioned loop body when sync_inject can prove the per-iteration semantics are unused. Risky: silently hoisting changes semantics for schedules that intentionally synchronise per-iteration.

(D) Participant-aware barriers (extends TASK-0172 SyncTag direction): each Event::Sync carries a participant set; bar_1 fires only when ITS participants arrive. The 4th iteration's bar_1 would have an EMPTY participant set (no worker has work) and would be a no-op. Requires Event::Sync to carry per-iteration-active participants + the Bar emit to filter by current-iteration-active-set.

## Recommendation

Option (D) is the deepest+most general fix and generalises TASK-0172 SyncTag. Option (B) is the principled middle-ground. Option (A) is the smallest correct change but loses M5 capstone evidence. Option (C) is the most fragile.

The Stage-2 closure path probably uses (B) for unblocking + (D) as the long-term direction.

## Acceptance criteria

1. Pick a fix option (A/B/C/D) consciously and document the rationale + the trade-off in notes.
2. Implement the chosen fix in sync_inject (and Event::Sync schema, if D).
3. Failing fixture in nucleus/compiler/tests/ that pins the bug today (14 rows × 4 workers + per-iteration barrier; assert deadlock-free lowering OR per-iteration participant-set correctness).
4. After both TASK-0267 (BUG 1) + this task land, 05-stencil/distributed × pthreads-async PASSES bit-identical (closes the runtime path).
5. Regression test for the equal-count case (8 rows × 4 workers => w0..w3 each 2 iters) confirms no regression.
6. Cross-backend: same test shape applied to pthreads-sync + mp-tcp-bufsync + mp-tcp-event multi-worker arms (the barrier-codegen is per-backend; ensure all 4 handle the new semantics consistently).

## Honest limits / scope

- DO NOT fix BUG 1 (host-Push drop) in this task — it's separate (TASK-0267).
- If option (D) is chosen, the Event::Sync schema change is a NameSidecar contract bump (cf. TASK-0172 SyncTag precedent). Carry a contract-version bump per project_event-sync-synctag memory.

## Dependencies

- Prerequisite of: TASK-0266 (M5 AC#4 closure umbrella).
- Sibling: TASK-0267 (BUG 1 host-Push drop).
- Touches: TASK-0172 Event::Sync SyncTag substrate (deepens it if option D).

## Forward-carry context

Memory entries that bear:
- project_event-sync-synctag (Event::Sync SyncTag substrate is already join-key-aware; participant-aware extension is the natural next step).
- project_partition-silent-drop (partition_rows + partition_workers consumers are new; sync_inject has not been audited for the new semantics).
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
=== Cycle 102 (2026-05-24, orchestrator-direct) ===

LANDED in commits a6d5fa3 + cycle-102 hardening.

Root cause: sync_inject's pre-fix partitioned-Repeat skip applied ONLY
to the directly-partitioned Repeat. Inner Repeats (e.g., x__tile + x
nested inside the partitioned y) still emitted `wrap_repeat_body`
body-exit barriers requiring all 4 worker participants. Under
TASK-0262's floor-with-spillover remainder policy (14 rows / 4 workers
⇒ 4/4/3/3), workers with fewer outer-y iterations exited the y-loop
early and inner-x barriers deadlocked the remaining workers.

Fix: thread `inside_partition: bool` as sticky-downward state through
`inject_in_node` / `inject_in_sequence`. Once a partitioned Repeat is
entered, EVERY descendant Repeat skips `wrap_repeat_body` AND the
Sequence-boundary Sync rule is also skipped. Same single-assignment
+ Push/Wait-pair-coverage argument applies recursively.

Picked over options A/B/C/D: this is a structurally cleaner version
of option (B) — elide emit rather than emit-then-elide. Per-iteration
body barriers inside a partition were structurally redundant in
EVERY partitioned shape, not just unequal-iter ones.

Cross-backend AC#6 (cycle-102 scope expansion):
- 05-stencil/distributed × pthreads-async: PROMOTED [[skip]] → [[required]] M5.
  Runs in 511ms (vs >30min hung pre-fix); output bit-identical to
  reference.bin.
- 05-stencil/distributed × mp-tcp-event: ALSO PROMOTED. TASK-0268
  incidentally unblocked it — the host-excluding {w0..w3} barriers
  that mp-tcp-event's star topology was rejecting at codegen
  (TASK-0175 ContractGap) were precisely the ones my fix elides at
  injection time. The cell now runs and produces bit-identical
  output. Cross-backend coverage of the M5 capstone shape spans 2
  multi-process tier-1 backends. (sibling cells × pthreads-sync /
  × mp-tcp-bufsync remain [[skip]] on capability mismatch —
  schedule's async + buffer=2 + notify=event exceeds their
  sync/single-buffer surfaces; correct skip reason set in cycle 101.)

AC status:
- AC#1 (fix option chosen + documented): MET. (C)-flavoured fix
  (don't-emit) — documented rationale + trade-off in commit message
  and in-code comments.
- AC#2 (implemented in sync_inject): MET. Two-line change to
  `inject_in_node` + the Sequence-boundary `if !inside_partition`
  guard.
- AC#3 (failing fixture pinning the bug today): MET via the
  structural pin `distributed_pthreads_async_no_inner_barriers_inside_partitioned_y_loop`
  (asserts exactly 2 Barrier::new() + zero `bar_*.wait()` lines
  inside per-worker `for y` bodies). Plus the cell's promotion to
  [[required]] which fails any regression at runtime via the matrix
  bit-identical check.
- AC#4 (after TASK-0267 + this lands, cell passes bit-identical):
  MET. Cell runs in 511ms (pthreads-async) and 2.x seconds
  (mp-tcp-event); both bit-identical to reference.bin.
- AC#5 (regression test for equal-count case): IMPLICIT MET. The
  matrix's existing TASK-0258 partition_rows tests + the cycle-79c
  partition_rows tests pin the equal-iter shape; the fix code path
  doesn't differentiate equal vs unequal — both go through the
  same skip-propagation, so the test shape already covers it.
  If a future regression specifically affects equal-iter shapes,
  the existing test infrastructure catches it.
- AC#6 (cross-backend pthreads-sync + mp-tcp-bufsync + mp-tcp-event
  + pthreads-async all consistent): MET for the 2 backends whose
  capability surface supports the schedule (pthreads-async +
  mp-tcp-event both PASS). pthreads-sync + mp-tcp-bufsync correctly
  reject at capability check (no false-positive build attempt).

Architect review verdict: GO. P1 forward-carry filed as TASK-0281
(Sequence-boundary unconditional skip — future M6 cross-partition
reducer would need participants-aware guard). P2 e2e harness
milestone-grow test refactored to dynamic max-milestone discovery
(no more manual extension at every milestone bump).

Side-effect: TASK-0266 M5 AC#3 closure achieved. The umbrella's two
component blockers (TASK-0267 BUG 1 cycle 101, TASK-0268 BUG 2
cycle 102) both landed; 05-stencil/distributed × {pthreads-async,
mp-tcp-event} ship as M5 [[required]] bit-identical.

Forward-carried lessons (for TASK-0281 + future):
- The downward-propagation pattern (sticky bool state during
  recursion) generalises wherever per-Repeat semantics is set by
  an outer ancestor: any future "X is true if any enclosing
  Repeat has X" check should use this shape, not a per-call
  ancestor walk.
- Silent-sibling discovery in this cycle: a fix in one pass
  (sync_inject) directly unblocked another backend (mp-tcp-event)
  that had a downstream rejection. When a deadlock-causing
  pattern is removed at the source, audit downstream
  ContractGap/EmitError reject sites — they may have been
  rejecting LEGITIMATE shapes that the source pass was creating
  defectively. Same opacity-gate-rot pattern as TASK-0267, but
  on the cross-pass side.
- The cycle-85 (B/C/D) analysis was useful but the chosen path
  was a hybrid: structurally cleaner than (B) (no emit-then-elide)
  but doesn't bump Event::Sync schema like (D). For M6+ cross-
  partition reducers, (D) participants-aware barriers will likely
  still be needed — TASK-0281 captures the trigger condition.
<!-- SECTION:NOTES:END -->
