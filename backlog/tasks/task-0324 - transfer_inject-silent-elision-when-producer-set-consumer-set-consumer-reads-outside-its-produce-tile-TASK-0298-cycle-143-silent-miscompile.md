---
id: TASK-0324
title: >-
  transfer_inject silent elision when producer-set == consumer-set + consumer
  reads outside its produce-tile (TASK-0298 cycle-143 silent-miscompile)
status: To Do
assignee: []
created_date: '2026-05-25 13:05'
labels:
  - compiler
  - transfer_inject
  - silent-miscompile
  - panic-not-diagnostic
  - M6
  - forward-carried-from-TASK-0298
dependencies:
  - TASK-0298
priority: medium
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Background

TASK-0298 cycle 143 investigation: a both-passes-distributed
schedule for 06-separable-filter
(`nuc-nucleus/examples/06-separable-filter/schedules/distributed2.sched.nuc`)
exposed a SILENT MISCOMPILE in transfer_inject.

## Schedule shape (the reproducer)

- pass 1 `hblur_acc` placed on `{w0..w3}`, `loop hy : partition=rows;`
- pass 2 `vblur_acc` ALSO placed on `{w0..w3}`, `loop vy : partition=rows;`
- `transfer tmp : sync;` declared.

## Algorithm-level data dependency

Pass 1 writes `tmp[hy][hx]` for hy in worker w_i's row-band.
Pass 2 reads `tmp[vm][vx]` where vm sweeps `0..H` for every (vy, vx).

Each consumer worker w_j (owning row-band [vy_lo..vy_hi]) needs
the ENTIRE `tmp` matrix to compute its output rows — the vm sweep
reads rows OUTSIDE w_j's own producer row-band.

## What transfer_inject emits (the defect)

`grep -n 'slot_' /tmp/task0298_pthreads_sync/src/main.rs` confirms
**zero slots allocated for `tmp`**. Only 8 slots total: 4 for
`in_arr` (host → workers) + 4 for `out` (workers → host). No
worker → worker `tmp` transfers exist in the emit; each worker's
`tmp` Vec holds only its own hy row-band, and pass 2's vm sweep
silently reads zeros from the non-owned rows.

The runtime artefact: `cmp output.bin reference.bin` reports
divergence at byte 129 (the first byte of row 2, the first output
row whose vertical taps reach row 4 — outside w0's row-band 0..4).

## Root cause hypothesis (preliminary)

When producer-worker-set == consumer-worker-set (both `{w0..w3}`),
transfer_inject's per-pair-tile machinery appears to treat each
worker's `tmp` access as same-worker (local) without checking
whether the consumer's read tile (`vm in 0..H`) FITS inside the
producer's write tile (`hy in [w_i_lo..w_i_hi]`). When the read
tile exceeds the producer's write tile, the missing rows are
NOT produced cross-worker transfers — they silently stay zero.

This is the N-to-N case the `transfer_inject.rs` module-doc
§"Honest limitations / N-to-M fan-out" warned about — but in
practice the fall-back behaviour is NOT "compute worker = dst"
(which would have at least emitted some transfer); it's SILENT
ELISION, which is the worst failure mode (no error, wrong output).

## Acceptance criteria

1. **Detection**: transfer_inject must detect when producer-set ==
   consumer-set AND the consumer's read tile on the partitioned-iv
   axis exceeds the producer's write tile on the SAME axis (i.e.,
   the consumer reads rows it did not produce locally).

2. **Diagnosis-first** (per
   [[feedback-panic-not-diagnostic-recurring]]): the FIRST
   landing step is to fail-loud with a typed `EmitError::
   ContractGap` ("data X requires cross-worker transfer in
   producer-set == consumer-set configuration; this transfer
   shape is not yet implemented; see TASK-0324") so silent
   miscompile becomes a compile error. This MUST land BEFORE any
   codegen attempt, even as a stub — the silent-miscompile
   exposure is the priority.

3. **Codegen** (the real fix): emit cross-worker `tmp` transfers
   for this shape. Simplest correct approach (N-to-N
   broadcast-of-gather):
   - Each producer w_i pushes its hy row-band of tmp to every
     other consumer w_j (4 producers × 4 consumers = 16 pairs,
     minus 4 self-pairs if locality is preserved; OR 16 with the
     self-pair as a no-op).
   - Each consumer w_j waits on 3 (or 4) row-band pushes and
     assembles them into its full tmp Vec.
   - Bit-identical against `reference.bin`.

4. **Smoke test**: the existing TASK-0298 schedule
   (`distributed2.sched.nuc`) becomes the smoke test. Add an e2e
   cell once codegen lands; remove the SILENT MISCOMPILE warning
   from the schedule's comment header.

5. **Defensive negative test**: add a fixture that constructs the
   prod-set == cons-set + reader-iv-exceeds-producer-tile shape
   and asserts the cycle-N typed error fires (AC#2 hardening).

## Honest scope

- **Severity**: HIGH (silent miscompile class is the worst
  failure mode; even a `panic!` would be better).
- **Exposure**: LOW today (no shipped cell triggers this; cycle
  143's investigation schedule is the only known reproducer).
- **Priority**: MEDIUM. AC#2 (diagnose-first) should land
  quickly to close the silent-miscompile window. AC#3 (codegen)
  can be deferred until an M6+ schedule actually needs it.

## Dependencies

- Blocks: TASK-0298 (kept In Progress as smoke-test target).
- Trigger for AC#3: an M6 or later schedule that legitimately
  needs both-passes-distributed shape.

## Cross-reference

- TASK-0298 cycle-143 final notes (the reproducer + evidence).
- `nucleus/nucleus-compiler/src/passes/transfer_inject.rs`
  module-doc §"Honest limitations / N-to-M fan-out" (the
  warning that was off by direction: not "compute worker = dst",
  but silent elision).
- MEMORY.md `feedback-panic-not-diagnostic-recurring` (the
  meta-pattern AC#2 follows).
- `nuc-nucleus/examples/06-separable-filter/schedules/distributed2.sched.nuc`
  (the reproducer; carries SILENT MISCOMPILE warning).
<!-- SECTION:DESCRIPTION:END -->
