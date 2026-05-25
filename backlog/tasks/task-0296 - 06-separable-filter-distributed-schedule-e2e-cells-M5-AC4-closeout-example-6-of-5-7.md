---
id: TASK-0296
title: >-
  06-separable-filter distributed schedule + e2e cells (M5 AC#4 closeout,
  example 6 of 5-7)
status: Done
assignee:
  - '@mped-orchestrator'
created_date: '2026-05-25 00:49'
updated_date: '2026-05-25 01:22'
labels:
  - M5
  - compiler
  - partition
  - distributed
  - 06-separable-filter
  - forward-carried-from-TASK-0294
dependencies: []
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background
PRD §11 M5 acceptance: "Examples 5–7 benefit measurably" from distributed/reuse. Currently only example 05-stencil ships distributed/distributed-2d/reuse schedules (cycle 115 closed). Example 06-separable-filter ships only naive + blocked. This task files the M5-extension distributed schedule for example 6.

## What example 6 looks like
Two-pass separable 5x5 box filter (see nuc-nucleus/examples/06-separable-filter/prog.algo.nuc):
- Pass 1: `for hy : 0..H { for hx : 0..W { for hk : 0..W { tmp[hy][hx] <-- hblur_acc(tmp[hy][hx], in_arr[hy][hk], hx, hk) } } }`
- Pass 2: `for vy : 0..H { for vx : 0..W { for vm : 0..H { out[vy][vx] <-- vblur_acc(out[vy][vx], tmp[vm][vx], vy, vm) } } }`

The algorithm uses the rectangular-accumulator pattern (every output visits every input position, kernel masks to the 5 clamp-to-edge taps). This avoids usize underflow at edges (TASK-0179 territory) but means the loops syntactically have full-array dependence.

## Honest scope concerns (READ BEFORE BRIEFING)
1. **Pass 1 (horizontal) is row-independent**: each hy row is computed only from in_arr[hy][...]. Row-band partition=rows on hy is sound, NO halo needed (no cross-row access). This is the natural distributed candidate.
2. **Pass 2 (vertical) is NOT row-independent**: vm iterates 0..H regardless of vy, so each output row reads ALL input rows of tmp. Standard halo inference (TASK-0260) cannot bound the dependency — `vm = 0..H` is full-array. Distributed pass 2 would need broadcast (full tmp at each worker), NOT halo strips.
3. Two viable shapes:
   - **(A) Pass-1-only distributed**: partition pass 1 on hy across {w0..w3}, keep pass 2 on a single worker (or on host). Smaller win, but proves M5 machinery generalises beyond example 5.
   - **(B) Both passes distributed**: pass 1 partitioned on hy (no halo), pass 2 partitioned on vy with BROADCAST of full tmp to every worker. Needs transfer_inject to handle broadcast-not-halo transfer, which may already be the default for cross-worker non-halo accesses.

Implementer decides which shape ships based on what the existing machinery supports today, scoping HONESTLY.

## Acceptance criteria
1. nuc-nucleus/examples/06-separable-filter/schedules/distributed.sched.nuc exists, lowers end-to-end, produces a tier-1 backend (likely pthreads-async to mirror 05/distributed) bit-identical to reference.bin.
2. New e2e cell in nuc-nucleus/e2e-matrix.toml: 06-separable-filter / distributed / pthreads-async (or whichever backend the schedule lands on) tagged milestone=M5, [[required]].
3. Sibling backends documented as [[skip]] with cited blockers (TASK-0042 capability, TASK-0175 mesh) — same pattern as 05/distributed.
4. Schedule comment explains the chosen partition shape and why (the honest scope concerns above).
5. e2e gate still green; baseline advances 96/80 → 97/81 (or higher if multiple backends carry the cell).

## Forward-carry from TASK-0294 cycle 115
Cycle 115 generalised backend-common/multi_worker_walker to handle 2D tiles. The 1D row-band pattern (which is what 06/distributed should likely use for pass 1) is the original pre-cycle-115 code path — well-trodden, no new walker work needed. The risk surface is in the algorithm-shape mismatch (rectangular accumulator vs halo inference), NOT in the codegen layer.

## Cross-references
- nuc-nucleus/examples/05-stencil/schedules/distributed.sched.nuc — template for the row-band pattern.
- nucleus/nucleus-compiler/src/passes/partition_rows.rs (TASK-0258) — the consumer.
- nucleus/nucleus-compiler/src/passes/halo_inference.rs (TASK-0260) — verify behaviour when access is `vm = 0..H` (full-array, NOT bounded halo).
- nucleus/nucleus-compiler/src/passes/transfer_inject.rs — verify it handles broadcast (whole-array cross-worker transfer) as well as halo strips.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
ORCHESTRATOR-DIRECT IMPLEMENTATION (cycle 116, per memory feedback-spawned-agents-refuse-code-edits).

Plan:
1. Read PRD §11 M5 + §6.3.3, prog.algo.nuc for 06, 05-stencil/distributed.sched.nuc (template), partition_rows + halo_inference + transfer_inject pass code.
2. Decide partition shape:
   - Pass 1 (hblur) is row-independent (each hy reads only in_arr[hy][...]). Row-band partition=rows on hy is natural.
   - Pass 2 (vblur) has vm = 0..H regardless of vy → full-array dependence → NOT halo-shaped. Investigate whether transfer_inject handles unbounded vm as broadcast.
3. Write nuc-nucleus/examples/06-separable-filter/schedules/distributed.sched.nuc with the chosen shape; documented schedule comment explaining the partition decision and any honest-scope limitations.
4. Run the full gate; iterate until bit-identical against reference.bin.
5. Add e2e cells to nuc-nucleus/e2e-matrix.toml; siblings as [[skip]] with cited blockers.
6. Verification gate green; commit; pass to read-only review.

Honest-failure path: if both shapes (A and B) hit codegen gaps, ship the simpler shape that works (e.g. pass-1 only on single worker, just to demonstrate the schedule lowers) and file follow-ups for the rest.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
CYCLE 116 PROGRESS (orchestrator-direct, 2026-05-25):
- distributed.sched.nuc written: pass 1 (hblur_acc) distributed across {w0..w3} with partition=rows on hy; pass 2 (vblur_acc) stays on host. All sync transfers (broadest backend reach). Chose this shape over both-passes-distributed because pass 2 has full-tmp dependency (vm iterates 0..H regardless of vy band); both-passes variant filed as TASK-0298 follow-up.
- 4 e2e cells added to nuc-nucleus/e2e-matrix.toml, all [[required]] M5: 06-separable-filter/distributed × {pthreads-sync, mp-tcp-bufsync, pthreads-async, mp-tcp-event}.
- 3 of 4 backends PASSED on first attempt. mp-tcp-bufsync FAILED bit-identical: output.bin all zeros.
- ROOT CAUSE: silent-sibling defect on render_wait_assign. mp-tcp-bufsync's Event::Wait emit rendered `{name} = {dec}` (whole-array overwrite) regardless of pair tile — so each worker's gather of its tmp row-band overwrote the whole tmp instead of pasting into the band, leaving tmp at the LAST-received worker's band (other bands zeroed). pthreads-async + mp-tcp-event already went via `backend_common::multi_worker_walker::render_wait_assign` (cycle 115 TASK-0294 + TASK-0117 leading-axis slice-paste), so they were correct.
- FIX: refactored render_wait_assign signature from (ctx: &WalkerCtx, ...) to (sidecar: &NameSidecar, pair_tiles: &BTreeMap<...>, ...) so non-walker backends can call it directly; updated the internal walker caller; lifted mp-tcp-bufsync's Wait emit to call it (+ Plan now carries pair_tiles via the shared collect_xfer_pairs helper). Byte-identical for pthreads-async + mp-tcp-event (signature change only, no logic change).
- VERIFIED FRESH: re-emitted mp-tcp-bufsync host.rs has `tmp[a..b].copy_from_slice(&_tmp[a..b])` slice-paste per recv, matching pthreads-async + mp-tcp-event.
- All 4 backends now PASS. e2e baseline advanced 96/80/0/16/0 → 100/84/0/16/0 (+4 cells, all M5 [[required]] bit-identical).

ALL ACs MET:
- AC#1: distributed.sched.nuc exists, lowers, bit-identical (4 backends).
- AC#2: 4 new e2e cells, all [[required]] M5.
- AC#3: no siblings needed [[skip]] documentation — all 4 backends carry the cell green.
- AC#4: schedule comment explains partition shape + honest scope + sync-vs-async rationale + bit-identicality argument.
- AC#5: baseline advanced 96/80 → 100/84 (4 cells).

GOTCHAS + FORWARD-CARRY:
- The render_wait_assign signature refactor was the smallest fix that lifts ALL backends onto the slice-aware path. A larger refactor (TASK-0284) would lift mp-tcp-bufsync entirely onto the shared multi_worker_walker — proper closure of this defect class. The current cycle did the minimal targeted fix.
- New silent-sibling memory: every backend's per-event walker that emits a Wait MUST go through render_wait_assign (or equivalently, the shared walker). pthreads-sync also has its own walker (it doesn't use the shared one — see partition_workers integration); audit pthreads-sync's single-worker Wait emit path too as a follow-up (TASK-0299 to file if relevant — to verify).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
CYCLE 116 CLOSEOUT (2026-05-25).

What landed:
- nuc-nucleus/examples/06-separable-filter/schedules/distributed.sched.nuc: pass 1 (hblur_acc) distributed across {w0..w3} with partition=rows on hy; pass 2 (vblur_acc) stays on host. All sync transfers (broadest backend reach).
- 4 [[required]] M5 cells in nuc-nucleus/e2e-matrix.toml: 06-separable-filter/distributed × {pthreads-sync, mp-tcp-bufsync, pthreads-async, mp-tcp-event}. ALL bit-identical PASS.
- nucleus/backend-common/src/multi_worker_walker.rs: render_wait_assign + wait_slice signature refactor (WalkerCtx → sidecar + pair_tiles).
- nucleus/backends/mp-tcp-bufsync/src/lib.rs: Plan carries pair_tiles; Event::Wait emit lifted from raw whole-array overwrite to shared render_wait_assign (silent-sibling defect closure).

Gate verified by qa-test-runner sub-agent (re-ran twice): 100/84/0/16/0, no flake. 844 unit tests dev + release. All 3 negative arms still bite. Architect sub-agent: GO (3 P1 follow-ups filed: TASK-0299/0300; 1 P1.3 wording fix applied in-cycle).

All ACs met:
- AC#1 distributed.sched.nuc exists, lowers, bit-identical (4 backends) ✓
- AC#2 4 new [[required]] M5 e2e cells ✓
- AC#3 no siblings needed [[skip]] (all 4 carry green) ✓
- AC#4 schedule comment explains partition shape + honest scope + backend reach + bit-identicality argument ✓
- AC#5 baseline advanced 96/80/0/16/0 → 100/84/0/16/0 (4 cells) ✓

Silent-sibling defect summary: 3 of 4 backends went via shared backend-common::multi_worker_walker::render_wait_assign (cycle 115 TASK-0294 + TASK-0117 leading-axis slice-paste); mp-tcp-bufsync had its own walker that emitted `{name} = {dec};` (whole-array overwrite), silently dropping partition-band slicing on the host gather. The 06/distributed × mp-tcp-bufsync cell was the first schedule exercising this code path on mp-tcp-bufsync (existing partition cells were either pthreads-sync only or had async-capability-mismatch SKIPs). Fix: refactor render_wait_assign signature to be callable without WalkerCtx, lift mp-tcp-bufsync's Event::Wait emit. Byte-identical preserved for pthreads-async + mp-tcp-event (verified by gate green twice across all their cells).

Follow-ups filed:
- TASK-0297 (07-matmul/distributed) — sibling M5 closeout, depends on TASK-0296.
- TASK-0298 (06 both-passes distributed via tmp broadcast) — N-to-M honest-scope follow-up.
- TASK-0299 (pin test for halo_widths[hblur_acc][hy]=0) — architect P1.1.
- TASK-0300 (lift pair_tiles into shared backend-common helper) — architect P1.2.

Lessons for future M5 closeout cycles:
1. A new schedule that exercises an UNUSED-IN-OTHER-BACKENDS code path WILL surface silent-sibling defects. The cycle-115 closeout filed TASK-0295 (sibling-promotion audit) precisely for this — cycle 116 hit one such audit organically by adding a new schedule that all 4 backends carry. Lesson: when a new tier-1 schedule lands, IMMEDIATELY check that every backend's Wait/Push emit goes through the shared helper.
2. partition=rows on a rectangular-accumulator algorithm (kernel reads all inputs, masks by position) is sound for the OUTER independent axis but does not bound the INNER full-range axis. The halo inference correctly produces halo=0 in this case (zero-offset accesses); the partition shape is independent of the kernel-mask semantics. Both checked structurally; no halo-strip Push/Wait synthesis fires.
3. Backend reach matters at M5 capstone: a sync-only schedule lowers on every tier-1 backend (4 cells per example) rather than just pthreads-async (1 cell). Use sync transfers when the M5 validation is the goal, not maximum runtime parallelism.
<!-- SECTION:FINAL_SUMMARY:END -->
