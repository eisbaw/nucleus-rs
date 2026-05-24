---
id: TASK-0268
title: >-
  sync_inject: participant-aware barriers for partitioned-loop bodies with
  unequal per-worker iteration counts
status: To Do
assignee: []
created_date: '2026-05-24 08:02'
updated_date: '2026-05-24 13:46'
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
=== Forward-carried from TASK-0267 (cycle 101, 2026-05-24) ===

TASK-0267 (BUG 1 — host-Push synthesis drop under block-governed
enclosing Repeat) LANDED in commit 336836f. The 05-stencil/
distributed × pthreads-async emit now correctly contains:
  ring_0..3.push(img_in.clone())  // 4 host-side Pushes at top of main()
(verified by `distributed_pthreads_async_host_pushes_img_in_to_every_worker`
in nucleus-compiler/tests/e2e_example_05.rs).

This task (BUG 2) is now the SOLE remaining blocker before the
05-stencil/distributed × pthreads-async cell can promote to
[[required]] bit-identical. The e2e-matrix.toml skip reason at
the cell (lines 695-699) was updated to cite TASK-0268 only.

Reproduction recipe (post-TASK-0267):
```
cd nucleus && nix develop --command bash -c '
  cargo run --release --bin nucleus -- \
    build --algo ../nuc-nucleus/examples/05-stencil/prog.algo.nuc \
          --sched ../nuc-nucleus/examples/05-stencil/schedules/distributed.sched.nuc \
          --backend pthreads-async \
          --kernels ../nuc-nucleus/examples/05-stencil/kernels.rs \
          --out /tmp/probe-bug2 &&
  cd /tmp/probe-bug2 && cargo build --release --quiet &&
  cp ../mark_thesis/nuc-nucleus/examples/05-stencil/input.bin input.bin &&
  timeout 30 env NUC_INPUT_PATH=input.bin NUC_OUTPUT_PATH=/tmp/out.bin \
    ./target/release/nuc-generated
'
```
Expected: hang. The 4 workers wait on `wX_ring_X.wait()` for
img_in (which is now correctly pushed by host — post-TASK-0267),
load their slice, and start iterating y. w0/w1 do 4 iterations
(rows 1..5 / 5..9); w2/w3 do 3 (9..12 / 12..15). At the 4th
iteration of w0/w1, bar_1/bar_2 fires — but w2/w3 have already
exited the loop. Workers w0/w1 block on the empty barrier.

Inspection: look at the emitted main.rs's per-worker sections
and grep for `bar_<N>.wait()` calls. They're inside the y-loop
body (each iteration), with 4 participants required — but the
loop range differs per worker, so the participant set at
iteration 4 is {w0, w1} not {w0..w3}.

Recommended fix option (per cycle-85 analysis): option (B)
trailing-partial-tile policy — emit one Repeat for the
divisible portion (12 rows = 4 × 3) and a separate trailing
Repeat for the remainder (2 rows), each with its own
worker-aware barrier participant set. Mirrors block_transform's
discipline. Option (D) participant-aware barriers is the deeper
generalisation; (B) is the principled middle-ground.

TASK-0267 pin to preserve (do NOT regress):
`distributed_pthreads_async_host_pushes_img_in_to_every_worker`
asserts the host emits 4 distinct `ring_<N>.push(img_in.clone())`
lines in main(). When you change sync_inject's barrier emit, the
host-Push synthesis MUST remain intact — the pin will fire LOUD
if you accidentally break Pass A's per-Wait classification.

Lesson from TASK-0267: when a deferral facility (here:
contains_block_inner opacity gate) blocks an M5 capstone, audit
whether the deferral predates a newer machinery (halo_widths,
partition_worker_ranges) that makes the deferral redundant.
Apply the same audit to sync_inject's barrier emit: does it
predate partition_worker_ranges (TASK-0212)? If yes, the
per-worker iteration count IS available — the fix is to consume
it at barrier emit time, not to add per-iteration probes.
<!-- SECTION:NOTES:END -->
