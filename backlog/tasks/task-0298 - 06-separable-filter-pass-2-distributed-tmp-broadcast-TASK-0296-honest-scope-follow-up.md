---
id: TASK-0298
title: >-
  06-separable-filter pass-2 distributed: tmp broadcast (TASK-0296 honest-scope
  follow-up)
status: In Progress
assignee:
  - '@mark'
created_date: '2026-05-25 01:12'
updated_date: '2026-05-25 13:21'
labels:
  - M5
  - compiler
  - partition
  - broadcast
  - 06-separable-filter
  - forward-carried-from-TASK-0296
dependencies:
  - TASK-0324
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Cycle 143 investigation outcome (AC#1 + AC#3 actioned, AC#2 deferred to TASK-0324)

### What was tested

Wrote experimental schedule `distributed2.sched.nuc` with both
hblur_acc and vblur_acc placed on `{w0..w3}` and both outer loops
(`hy`, `vy`) partition=rows. Compiled for pthreads-sync (cargo
release build of the nucleus driver). Compilation succeeded after
the missing `transfer out : sync;` was added (the compiler
correctly caught the host-back gather requirement for save_image).

### What the compiler produced

`grep -n 'slot_' /tmp/task0298_pthreads_sync/src/main.rs` shows
only 8 slots: 4 for `in_arr` (host → workers) + 4 for `out`
(workers → host). **Zero slots for `tmp`** — transfer_inject
silently elided every cross-worker `tmp` transfer pass 2 needs.

Each worker computes its hy row-band of tmp, then directly uses
its own (partial) tmp Vec in pass 2 where `vm` sweeps 0..H. Rows
of tmp outside w_i's own band stay zero, and vblur_acc adds zero
for those clamped-tap contributions.

### Runtime evidence (the bite)

```
cd /tmp/task0298_pthreads_sync
bash run.sh > output.bin
cmp output.bin reference.bin
  → output.bin reference.bin differ: byte 129, line 1
```

Row 0..1 OK (the 5 vertical taps clamp to vm ∈ {0,1,2} for y=0
and {0,1,2,3} for y=1 — all within w0's row-band 0..4). Row 2
first to diverge — its taps reach vm=4, the first row in w1's
band that w0's tmp doesn't have.

### AC outcome per TASK-0298

- **AC#1** (investigate): DONE. transfer_inject does NOT handle
  the producer-set == consumer-set + reader-iv-exceeds-produce-tile
  shape. The "N-to-M fan-out" warning in transfer_inject.rs is
  off by direction — actual behaviour is silent elision, not
  "compute worker = dst" fall-back.
- **AC#2** (if works: add cell + bit-identical): N/A — broadcast
  does NOT work as-is.
- **AC#3** (if fails: file precise gap): DONE — filed as
  TASK-0324 (Medium, dependency TASK-0298), with the reproducer
  + runtime evidence + a diagnose-first AC (per
  feedback-panic-not-diagnostic-recurring) that prioritises
  closing the silent-miscompile window over the full codegen
  extension.
- **AC#4** (naming + placement): the experimental schedule
  remains at `distributed2.sched.nuc` with a STRONG top-of-file
  warning marking it as the SILENT MISCOMPILE smoke-test target
  for TASK-0324. Not added to e2e-matrix.toml (would silently
  fail the gate if added — and the cell isn't ready until
  TASK-0324 lands).

### Status: kept In Progress

Per TASK-0298 AC#3, this task stays In Progress as the smoke-test
target for TASK-0324. Closes when TASK-0324's AC#4 lands the
bit-identical e2e cell.

### Gates

Doc-only artefact change (one new experimental schedule file with
warning header + one new tracker task md). No production code
touched in this cycle. Existing gates unchanged:
- `just test` (dev): 874/0/3 (cycle 142b baseline preserved).
- `just test-release`: 874/0/3.
- `just e2e`: 108/92/0/16/0.
(Gate not re-run in this cycle because no compiled-source change
was made; will re-run during the review-gate verification.)

### Cycle gotchas / forward-carried lessons

1. **Silent miscompile from transfer_inject** is a real failure
   class — not just slow / inefficient but PRODUCES WRONG OUTPUT.
   Any future M5+/M6+ schedule that distributes a producer/consumer
   pair on the SAME worker set must explicitly test bit-identical
   against reference.bin; relying on "the compiler accepted it"
   is unsafe today.

2. The transfer_inject.rs §"Honest limitations / N-to-M fan-out"
   doc is itself a doc-lie candidate — it described the fallback
   as "compute worker = dst", but the actual fallback is silent
   elision. Update during TASK-0324 to match reality.

3. The reproducer pattern (write the schedule, run nucleus
   driver, grep slot_ in emitted main.rs, actually run the binary
   and cmp against reference.bin) is the cleanest way to surface
   silent miscompiles. Promoted to a forward-carried lesson on
   TASK-0324 AC#5 (defensive negative test fixture).

## Cycle 143 review-gate outcome + in-thread fold-back

### qa-test-runner: GO

Independently reproduced all gate numbers (dev 874/0/3, release
874/0/3, e2e 112/92/0/20/0). Confirmed the reproducer reproduces:
`cmp` reports divergence at byte 129 (1-based) == offset 128
(0-based), and `grep slot_` confirms zero allocations for `tmp`.
Only P3: byte 128 vs 129 inconsistency between commit body and
TASK-0324 desc (1-based vs 0-based base — semantically equivalent,
cosmetic).

### mped-architect: GO with three P2s (all folded back in-thread
into TASK-0324 description rewrite)

- **P2-1 root-cause precision**: cycle-143's framing of the
  defect as "per-pair-tile machinery treats each worker's access
  as same-worker without checking..." was IMPRECISE. The actual
  elision is at
  `nucleus/nucleus-compiler/src/passes/transfer_inject.rs:2501-2503`
  — a `BTreeSet` set-equality `continue` that NEVER reaches tile
  construction. TASK-0324 description rewritten with the precise
  citation + the actual code text.

- **P2-2 silent sibling at 13-cnn-inference/batch_parallel**:
  conv_block_1/conv_block_2/classifier all on {w0..w3}; feat1
  and feat2 have producer-set == consumer-set. The line-2501
  continue fires for both — identical code path to 06/distributed2.
  Today correctness-safe (reader iv n == partition iv n) but the
  silent-elision path is unguarded. The current 13-cnn skip cites
  an UNRELATED reason (TASK-0042 partition=workers gap), so the
  silent-elision class is double-masked. SEVENTH firing of the
  cycle-128 silent-sibling meta-rule in this session-chain.
  TASK-0324 rewrites to include a first-class sibling section +
  AC#5 sibling-guard test.

- **P2-3 doc-lie magnitude understated**: cycle-143 commit body
  called the transfer_inject.rs:82-90 module-doc paragraph
  "off by direction". Architect correction: the "compute worker
  = dst" fallback DOES NOT EXIST in the code; the doc fabricates
  a behaviour. The actual code path is `continue; no transfer`.
  Promoted to TASK-0324 AC#0 (doc-lie fix) at the front of the
  AC list.

### P3-1 byte-offset reconciliation (in-thread)

Cycle-143 commit body said "offset 128"; TASK-0324 description
v1 said "byte 129". Both correct (cmp 1-based vs offset 0-based,
same byte). TASK-0324 description v2 normalizes to "cmp 1-based
byte 129 (== 0-based offset 128)" throughout.

### e2e-matrix skip reasons: no change needed

Skip reasons already used "first byte diverges at offset 128"
(0-based), consistent with the commit body. The TASK-0324
description was the only outlier; now normalized.

### Cycle conclusion

No production code change in cycle 143; the cycle's value is
the investigation outcome + filed TASK-0324 + the
architect-driven precision corrections to that filing. TASK-0298
stays In Progress per its AC#3.

Cycle-143 demonstrates the parallel review gate working exactly
as designed: the implementer's narrative had three concrete
imprecisions (one root-cause, one sibling-sweep gap, one doc-lie
magnitude) that the read-only architect caught at review time.
All three resolved in-thread before any implementer picks up
TASK-0324 — the next subagent reads a correct description from
line 1.
<!-- SECTION:NOTES:END -->
