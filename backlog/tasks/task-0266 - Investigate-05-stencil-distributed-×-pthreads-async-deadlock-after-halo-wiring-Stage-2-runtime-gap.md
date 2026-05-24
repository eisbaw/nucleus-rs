---
id: TASK-0266
title: >-
  Investigate 05-stencil/distributed × pthreads-async deadlock after halo wiring
  (Stage-2 runtime gap)
status: To Do
assignee: []
created_date: '2026-05-24 04:04'
updated_date: '2026-05-24 07:29'
labels:
  - M5
  - bug
  - compiler
  - deadlock
  - stage-2
dependencies:
  - TASK-0262
  - TASK-0263
priority: high
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

Cycle 83 landed TASK-0262 (floor-with-spillover remainder policy for partition_rows/partition_workers) and TASK-0263 (transfer_inject consumes halo_widths to extend per-tile transfer ranges). The intent was to close M5 (TASK-0043) AC#4 by promoting 05-stencil/distributed × pthreads-async to [[required]] and verifying bit-identical to reference.bin.

## Observed failure

Cell promoted to [[required]]; `just e2e` started; the generated nuc-generated binary (PID 3309558) HUNG indefinitely. No progress past 30 minutes; the e2e harness was stalled on cell 39 of 88 ("05-stencil | distributed | pthreads-async ..." — no PASS/FAIL printed). Killed manually.

The cycle-83 implementer's intermediate commits stay (624d7dc + cf2f9ac): the partitioning + halo-extended tile arithmetic are CORRECT in isolation; the runtime semantic of the synthesised halo transfers is the gap.

## Candidate root causes

1. **Circular Push/Wait dependency between adjacent workers.** Each worker w_i now Pushes its boundary rows to w_{i-1} and w_{i+1} (and Waits for theirs). If the Push/Wait ordering across the workers forms a cycle, the Condvar coordination in pthreads-async's Ring<T> deadlocks. transfer_inject's fan-out per-(src,dst) pair may have generated an ordering that interlocks.
2. **Off-by-one in tile-with-halo boundary arithmetic.** The kernel blur3 reads grid[y-1], grid[y], grid[y+1]. At y=lo_i (the bottom of worker w_i's band), grid[y-1] is in w_{i-1}'s band. The halo extension should have shipped that row, but if the per-worker partition_worker_ranges is now [lo_i - halo, hi_i + halo) (inclusive halo expanded into the read range) AND the kernel still reads grid[y-1] expecting the absolute index, the per-worker tile may already cover the read — OR the absolute-index rebinding may double-add the halo.
3. **Missing halo-strip Push at the partition seam.** transfer_inject extends the tile bounds but may not have added the Push/Wait pairs that ship the halo rows. The tile-extension changes which rows the worker READS but does not by itself synthesise the cross-worker transfer; the Push is implicit in the host-to-worker fan-out today, which assumes whole-array push (TASK-0117). For partitioned workers, the host's tile is the FULL grid, but each worker now waits for its (extended) row band — the host's transfer must include the halo rows for adjacent bands.

## What's needed

1. Reproduce the hang in a controlled fixture (smaller grid, e.g. H=4, W=4, 2 workers, halo=1).
2. Use NUC_TRACE=1 or strace on the worker threads to identify whether the hang is in Push (waiting for ring space), Wait (waiting for arrival), or compute (infinite loop in the kernel).
3. Based on the trace:
   - (a) If Push/Wait cycle: change transfer_inject's per-pair fan-out order OR introduce a topological ordering on the halo Push/Wait pairs.
   - (b) If off-by-one: inspect the emitted main.rs at `nucleus/target/e2e-matrix/run-*/05-stencil__distributed__pthreads-async/src/main.rs` and trace the per-iteration index arithmetic against the kernel signature.
   - (c) If missing halo strip: extend transfer_inject to synthesise additional cross-worker XferPlaceholders for the halo rows, not just extend the existing whole-array push's range.

## Acceptance

1. The 05-stencil/distributed × pthreads-async cell promotes from [[skip]] to [[required]] and PASSES bit-identical to reference.bin (sha256: read from examples/05-stencil/reference.bin).
2. `just e2e` total 88 / 74 pass / 0 fail / 14 skip / 0 required-fail.
3. `just determinism-check` continues to PASS (byte-identical re-emit).
4. Root cause documented + tests added that pin the fix (regression test for the deadlock; positive test for the halo-strip cross-worker Push/Wait shape).

## Dependencies
- TASK-0262 (remainder policy) — landed.
- TASK-0263 (transfer_inject halo extension) — landed.
- TASK-0260 (halo Stage 1) — landed.

## Forward-carry context
This is the closing keystone for M5 AC#4 (TASK-0043). The full M5 differential matrix on examples 5/6/7 distributed depends on this task closing. Until it lands, M5 AC#4 stays partial.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
ORCHESTRATOR ROOT-CAUSE DIAGNOSIS (cycle 83 post-PROBE-FAILED inspection of the emitted code at /home/mpedersen/topics/mark_thesis/nucleus/target/e2e-matrix/run-3307562-*/05-stencil__distributed__pthreads-async/src/main.rs):

The deadlock is NOT in halo Push/Wait pairs (those work — the host's transfer_inject extension correctly ships halo-extended slices to each worker, verified at main.rs:80,105,130). The deadlock is in PER-ITERATION BARRIERS firing inside the partitioned y-loop body, combined with the floor-with-spillover policy producing UNEQUAL iteration counts:

- TASK-0262 policy: 14 rows / 4 workers = floor-with-spillover ⇒ w0=4 rows, w1=4 rows, w2=3 rows, w3=3 rows.
- Emitted main.rs shows w0_bar_1.wait() and w0_bar_2.wait() fire INSIDE the y-loop body.
- bar_1 + bar_2 are sync_inject barriers requiring ALL 4 workers to participate.
- w0/w1 call bar_1 four times; w2/w3 call it three times.
- On the 4th iteration, w0/w1 wait for w2/w3 — who are already past the loop — INDEFINITELY.

This is a structural problem at the partition_rows × sync_inject seam: when partition=rows produces unequal per-worker iteration counts, the in-body barriers expect equal counts and deadlock.

## Fix options (pick consciously)

(A) **Restore NonDivisible reject for partition_rows**: revert TASK-0262's floor-with-spillover, return to the divisible-only constraint. The 14-row range of 05-stencil/distributed becomes a compile-time reject — fail-fast discipline, but the example schedule cannot demonstrate row-band partitioning unless the algorithm changes (H=16 → H=17 makes y∈1..16 length 15 = 3×5, divisible by some smaller worker counts but not 4).

(B) **Trailing-partial-tile policy** (TASK-0262 option c): mirror block_transform's discipline — emit one Repeat for the divisible portion (12 rows = 4 × 3 for our case) and a separate trailing Repeat for the remainder (2 rows), each with its own worker-aware barrier participant set. Requires sync_inject to be aware of the trailing-partial split.

(C) **Hoist per-iteration barriers out of the partitioned loop body**: if sync_inject can prove a barrier's participants all execute the same number of iterations, fine; otherwise hoist the barrier ABOVE the partitioned y-loop OR omit it. Requires sync_inject to consult partition_worker_ranges.

(D) **Participant-aware barriers** (TASK-0172 SyncTag direction): each Event::Sync carries a participant set; bar_1 fires only when ITS participants arrive. The fourth iteration's bar_1 would have an EMPTY participant set (no worker has work) and would be a no-op. Requires Event::Sync to carry participants + the Bar emit to filter by current-iteration-active-set.

## Recommendation

Option (A) is the smallest correct change but loses the M5 capstone example. Option (B) is the principled long-term fix. Option (C) is a partial fix (silently hoisting barriers changes semantics for schedules that intentionally synchronise per-iteration). Option (D) is the deepest fix but generalises TASK-0172.

For closing M5 AC#4: Option (A) for THIS task + a follow-up to land Option (B) for the actual unblocking of the 4-way distributed shape.

## Forward-carry

Until this lands:
- 05-stencil/distributed × pthreads-async stays [[skip]] with TASK-0266 reason.
- TASK-0043 (M5 capstone) AC#4 (e2e differential green on 5/6/7 distributed) stays partial.
- TASK-0262 + TASK-0263 commits are LEGITIMATE Stage-2 progress; they don't need reverting — the codegen is correct, the runtime gap is the partition×barrier seam.

REFINED DIAGNOSIS (cycle-84 attempt, 2026-05-24):

Cycle 84 attempted to close M5 AC#4 by narrowing the 05-stencil/distributed schedule to 2 workers (14/2=7 exact, sidesteps the alleged unequal-iteration-count theory from cycle-83). The 2-worker variant ALSO HUNG. Killed the new run at PID 3329456.

Inspection of the 2-worker emit at /home/mpedersen/topics/mark_thesis/nucleus/target/e2e-matrix/run-3327453-*/05-stencil__distributed__pthreads-async/src/main.rs identified the ACTUAL root cause:

HOST FAILS TO EMIT PUSH FOR img_in.
- Line 115:  — host loads input.
- Lines 116-117:  — host hits barriers.
- Lines 118-119: ,  — host receives img_out from workers.
- MISSING:  and  — the host's send of img_in to w0 and w1.
- Workers w0, w1: line 76/101  /  — wait for img_in INDEFINITELY.

The cycle-83 unequal-iteration-count theory was WRONG. The deadlock is in the host-side Push synthesis for partitioned schedules, NOT in the per-iteration barrier participation count.

## Where the bug likely lives

transfer_inject's host-side Push synthesis path (in passes/transfer_inject.rs). The cycle-83 commit cf2f9ac extended transfer_inject to consume halo_widths — it adds halo extension to existing XferPlaceholder tiles but may have inadvertently dropped the host's whole-array Push synthesis for partitioned consumers OR the host's Push synthesis was always missing for partition_rows (since partition_rows is new and never had downstream consumer testing).

Likely candidates:
1. The host's emission walks ACFG XferPlaceholders for OPs placed on host. With partition_rows, the host's Op is  (a producer for img_in). The Push synthesis for the producer side may not be generating Push events when the consumer is partitioned across multiple workers.
2. cf2f9ac's extend_xfer_tiles_for_halo may have re-shaped the XferPlaceholders in a way that broke the host-walker's recognition of which Xfers need a host-side Push.
3. Pre-existing partition_rows + transfer_inject gap: TASK-0258 partition_rows wrote per-worker ranges into partition_worker_ranges but transfer_inject was never updated to emit per-worker Push events from the host. The whole-array Push was assumed to be the right thing — but when consumers READ partitioned slices, the host must still push the whole array, AND each worker must consume its slice.

## Verification

The simplest fix-or-diagnose probe:
- Build the schedule WITHOUT cf2f9ac (revert it temporarily) and check if the host still fails to emit Push. If yes → pre-existing partition_rows gap (NOT cycle-83 regression). If no → cf2f9ac broke something downstream.
- This isolation is the next session's first step.

## Updated fix-options for closure

(E) **Fix host-side Push synthesis for partitioned consumers** — the actual root cause. transfer_inject needs to emit Push events from the host (producer of img_in) targeting EACH consumer worker w_i, with the (possibly halo-extended) slice of img_in that w_i needs. The cycle-83 work on the receive-side tile arithmetic is correct; the send-side (host's Push) is what's missing.

Options A-D from the earlier diagnosis are SUBORDINATE: they are about runtime semantics (barrier participation, iteration counts) which are downstream of the host actually shipping img_in. Until E lands, A-D don't matter.

## Honest stop signal

Cycle 84 is closed without closing TASK-0266. The cycle's value: REFINED the diagnosis from 'unequal-iteration barrier deadlock' (wrong) to 'host fails to emit Push for partitioned consumers' (precise, verified by emit-inspection). The next session picks this up cold with the diagnosis recorded and the verification probe (build without cf2f9ac) documented.

M5 AC#4 stays BLOCKED on TASK-0266 with the new precise root cause.

NOTE-FIX (shell expansion ate the prior diagnosis): the cycle-84 attempt confirmed the deadlock root cause is HOST FAILS TO EMIT PUSH for img_in to partitioned worker consumers. In the emitted main.rs the host loads img_in via load_image then waits on barriers + receives img_out, but never invokes ring_0.push() or ring_1.push() to ship img_in to the workers. Workers w0/w1 are blocked on w0_ring_0.wait() and w1_ring_1.wait() indefinitely. This is NOT the cycle-83 unequal-iteration-count barrier deadlock; the bug is in transfer_inject host-side Push synthesis for partition_rows consumers. Next-session probe: temporarily revert cf2f9ac and re-run to determine if it is a cycle-83 regression or a pre-existing partition_rows + transfer_inject gap that was masked by the [[skip]]. Fix option E (the real fix): make transfer_inject emit per-consumer host-side Push for partitioned consumers; options A-D from the earlier diagnosis are downstream of E.
<!-- SECTION:NOTES:END -->
