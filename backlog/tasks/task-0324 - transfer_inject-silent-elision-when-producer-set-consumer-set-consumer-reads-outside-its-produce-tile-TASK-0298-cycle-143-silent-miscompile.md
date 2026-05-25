---
id: TASK-0324
title: >-
  transfer_inject silent elision when producer-set == consumer-set + consumer
  reads outside its produce-tile (TASK-0298 cycle-143 silent-miscompile)
status: To Do
assignee: []
created_date: '2026-05-25 13:05'
updated_date: '2026-05-25 13:20'
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
divergence at cmp 1-based byte 129 (== 0-based offset 128), the
first byte of row 2. With H=W=16 and i32 (4 bytes), row stride =
64 bytes → row 2 starts at offset 128. Row 2 is the first output
row whose vertical taps reach row 4 — outside w0's row-band 0..4.

## Root cause (PRECISE — cycle-143 architect P2-1 correction)

The elision happens at a `BTreeSet` set-equality short-circuit,
BEFORE any tile or reader-iv analysis. From
`nucleus/nucleus-compiler/src/passes/transfer_inject.rs:2501-2503`:

```
if producer_workers == &consumer_workers {
    continue; // Same entity — intra-worker dataflow.
}
```

The `continue` skips the entire TASK-0117 fan-out cartesian-
product loop at lines 2544-2559. The pass NEVER REACHES tile
construction or reader-iv inspection for this case. Cycle-143's
initial framing ("the per-pair-tile machinery treats each
worker's access as same-worker without checking whether the
consumer's read tile fits the producer's write tile") was
imprecise — there is no tile-aware code path here at all to
"not check"; the path is `continue; no transfer`.

Why the existing 05-stencil/distributed schedule does NOT
trigger this: 05's halo source (host load_image) and dest
(workers) DIFFER as sets, so line 2501 never fires for img_in;
likewise img_out (workers → host save_image). 06/distributed2
is the first in-tree schedule with producer-set == consumer-set
AND the consumer reading outside its own slice.

## Silent sibling: 13-cnn-inference/batch_parallel
(cycle-143 architect P2-2 — currently MASKED, latent footgun)

`nuc-nucleus/examples/13-cnn-inference/schedules/batch_parallel.sched.nuc:17-22`
places `conv_block_1`, `conv_block_2`, `classifier` ALL on
`{w0..w3}` with `loop n : partition=workers;`.
`nuc-nucleus/examples/13-cnn-inference/prog.algo.nuc:58-60`:

```
feat1[n]  <-- conv_block_1(input[n]);
feat2[n]  <-- conv_block_2(feat1[n]);
output[n] <-- classifier(feat2[n]);
```

Producer-set == consumer-set on `feat1` AND `feat2`. The
line-2501 continue fires for both — IDENTICAL code path to 06/
distributed2. Today this is correctness-safe because reader iv
`n` IS the partition iv (`partition=workers` on `n`), so each
consumer reads exactly its own slice. BUT:

1. The silent-elision code path fires identically.
2. The current `[[skip]]` for 13-cnn/batch_parallel (e2e-matrix.toml
   ~lines 464-499) cites an UNRELATED reason (TASK-0042 partition=
   workers gap), so the silent-elision class is double-masked.
3. Any future shift / halo / cross-batch reuse variant on
   13-cnn would silently miscompile with no e2e signal.
4. Once TASK-0042 lifts (unblocks 13-cnn/batch_parallel), the
   latent unguarded path becomes a hidden footgun whose
   correctness depends on a coincidence between reader-iv and
   partition-iv.

This is the cycle-128/138/140/141/142/142b/143 silent-sibling
meta-rule firing for the SEVENTH time. The cycle-143 implementer
did NOT search for siblings before filing TASK-0324; the gap was
caught by the architect's read-only review.

## Acceptance criteria

### AC#0: doc-lie fix (cycle-143 architect P2-3)

Fix the doc-lie at `transfer_inject.rs:82-90`:

```
//! - **N-to-M fan-out** (both sides multi-worker, e.g. an all-to-all
//!   shuffle) falls back to the "compute worker = dst" convention
//!   when constructing per-pair tiles.
```

The "compute worker = dst" fallback DOES NOT EXIST for this case.
The structural code path is line 2501-2503's `continue; no
transfer` — the pass never reaches per-pair-tile construction.
Cycle-143 commit body called this "off by direction"; architect
P2-3 correction: the doc fabricates a fallback that does not
exist. Rewrite the paragraph to honestly describe the actual
short-circuit + cite the line numbers + cross-reference
TASK-0324.

### AC#1: detection logic

Detect when `producer_workers == &consumer_workers` AND the
consumer's read tile on any non-partition axis would require
slices the local producer does NOT own (i.e., the consumer's
read iv differs from the partition iv on a partitioned axis, or
the consumer's tile bounds exceed the producer's tile bounds).
Equivalent observation: when reader-iv == partition-iv on every
shared axis, the elision is correctness-safe (13-cnn case);
otherwise it is a silent miscompile (06/distributed2 case).

### AC#2: diagnose-first fail-loud guard

Per [[feedback-panic-not-diagnostic-recurring]], the FIRST
landing step is to fail-loud with a typed
`EmitError::ContractGap` ("data X in same-worker-set producer/
consumer where consumer reads outside its produce-tile; this
cross-worker transfer shape is not yet implemented; see
TASK-0324") right at the line-2501 short-circuit. This MUST
land BEFORE any codegen extension — silent-miscompile exposure
is the priority, and a typed error is strictly better than
wrong output even if it temporarily breaks more cells.

The guard MUST be precise enough that 13-cnn/batch_parallel's
correctness-coincides case does NOT spuriously fire. Use the
reader-iv == partition-iv check (or equivalent) to discriminate.

Note: per [[feedback-cross-pass-silent-sibling]], adding
ContractGap rejections has historically unblocked LEGITIMATE
shapes elsewhere (TASK-0268 / TASK-0175). The AC#2 guard must
be measured against the existing pass / skip cell matrix to
confirm no shipped cell newly breaks.

### AC#3: codegen extension

Emit cross-worker `tmp` transfers for this shape. Simplest
correct approach (N-to-N broadcast-of-gather):

- Each producer w_i pushes its hy row-band of tmp to every other
  consumer w_j (4 producers × 4 consumers = 16 pairs, minus 4
  self-pairs if locality is preserved; OR 16 with the self-pair
  as a no-op).
- Each consumer w_j waits on 3 (or 4) row-band pushes and
  assembles them into its full tmp Vec.
- Bit-identical against `reference.bin`.

### AC#4: smoke test promotion

The existing TASK-0298 schedule
(`distributed2.sched.nuc`) becomes the smoke test. Add an e2e
cell once codegen lands; remove the SILENT MISCOMPILE warning
from the schedule's comment header AND remove the four
[[skip]] entries from `nuc-nucleus/e2e-matrix.toml`
(~lines 1265-1304, all citing TASK-0324) AND add four
[[required]] entries in their place.

### AC#5: defensive negative + sibling guard tests

- Add a fixture that constructs the prod-set == cons-set + reader-
  iv-exceeds-producer-tile shape and asserts the cycle-N typed
  error fires (AC#2 hardening).
- Add a fixture that constructs the prod-set == cons-set + reader-
  iv == partition-iv shape (the 13-cnn case) and asserts the
  guard does NOT fire (the correctness-coincides escape valve).

## Honest scope

- **Severity**: HIGH (silent miscompile class is the worst
  failure mode; even a `panic!` would be better).
- **Exposure**: LOW today (no shipped cell triggers this
  defectively; the 13-cnn correctness-coincides case is sound
  by accident but masks the silent-elision path).
- **Priority**: MEDIUM. AC#0 + AC#2 should land quickly to close
  the silent-miscompile window. AC#3 (codegen) can be deferred
  until an M6+ schedule actually needs it.

## Dependencies

- Blocks: TASK-0298 (kept In Progress as smoke-test target).
- Trigger for AC#3: an M6 or later schedule that legitimately
  needs both-passes-distributed shape, OR the eventual M5+
  schedule that lifts TASK-0042 on 13-cnn/batch_parallel
  variants.

## Cross-reference

- TASK-0298 cycle-143 final notes (the reproducer + evidence).
- `nucleus/nucleus-compiler/src/passes/transfer_inject.rs:2501-2503`
  (the precise silent-elision site).
- `nucleus/nucleus-compiler/src/passes/transfer_inject.rs:82-90`
  (the doc-lie about a fictional "compute worker = dst"
  fallback — AC#0 target).
- `nuc-nucleus/examples/13-cnn-inference/schedules/batch_parallel.sched.nuc:17-22`
  + `nuc-nucleus/examples/13-cnn-inference/prog.algo.nuc:58-60`
  (silent sibling, currently masked by TASK-0042 skip).
- `nuc-nucleus/e2e-matrix.toml` ~lines 464-499 (the TASK-0042
  skip masking the 13-cnn sibling).
- `nuc-nucleus/e2e-matrix.toml` ~lines 1265-1304 (the four
  distributed2 skips filed cycle 143).
- `nuc-nucleus/examples/06-separable-filter/schedules/distributed2.sched.nuc`
  (the reproducer; carries SILENT MISCOMPILE warning).
- MEMORY.md `feedback-panic-not-diagnostic-recurring` (the
  meta-pattern AC#2 follows).
- MEMORY.md `feedback-silent-sibling-defect` (the meta-rule
  whose 7th firing in this thread caught the 13-cnn sibling at
  review-gate time, not filing time).
- MEMORY.md `feedback-cross-pass-silent-sibling` (the precedent
  for ContractGap-unblocks-legitimate-shapes; informs AC#2
  rollout strategy).
- MEMORY.md `feedback-comment-doc-lie-recurring` (the meta-
  pattern AC#0 closes).

## Cycle-143 architect-review fold-back appendix

The original cycle-143 filing of this task contained three
imprecisions caught by the cycle-143 architect review (P2-1,
P2-2, P2-3) and corrected in-thread before any implementer
picked the task up:

- **P2-1 root-cause precision**: original "the per-pair-tile
  machinery treats each worker's access as same-worker without
  checking whether the consumer's read tile fits the producer's
  write tile" rewritten to the actual line-2501 set-equality
  short-circuit. The pass never reaches tile construction.
- **P2-2 sibling sweep gap**: original filing did not mention
  13-cnn-inference/batch_parallel; architect P2-2 found the
  identical code path firing there masked by the TASK-0042 skip.
  Added as a first-class section + AC#5 sibling-guard test.
- **P2-3 doc-lie magnitude**: original called the
  transfer_inject.rs:82-90 paragraph "off by direction";
  architect P2-3 correction: the "compute worker = dst" fallback
  is fabricated, not directionally wrong. Promoted to AC#0.
- **P3-1 byte-offset reconciliation**: cycle-143 commit body
  said "offset 128"; this description said "byte 129". Both
  correct (cmp 1-based vs offset 0-based, same byte); normalized
  to "cmp 1-based byte 129 (== 0-based offset 128)" throughout.

The fold-back preserves the original AC numbering intent but
adds AC#0 (doc-lie) at the front. Implementer onboarding should
read THIS rewritten description, not look for a separate "v1"
artifact.
<!-- SECTION:DESCRIPTION:END -->
