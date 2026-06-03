---
id: TASK-0432
title: >-
  TASK-0430 follow-up: distributed textbook scatter + dedicated
  unconstrained-input fixture
status: Done
assignee:
  - '@me'
created_date: '2026-06-02 23:43'
updated_date: '2026-06-03 01:39'
labels:
  - compiler
  - scatter
  - histogram
  - broaden
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Two BROADEN follow-ups to TASK-0430 (X1 pure-call-in-index, single-worker 08-histogram/textbook). (1) DISTRIBUTED textbook scatter: partition the input i loop across workers, per-worker private partial histograms (whole-array replicate, the data-dependent write index marks histograms dim OPAQUE), host element-wise-sum combine - the analog of distributed.scatter.sched.nuc (TASK-0384) but with the bucket() call in index position. Soundness same as the bounded distributed scatter (partition over input index i, never over bins). (2) DEDICATED UNCONSTRAINED-INPUT FIXTURE: the landed textbook example shares 08-histograms input.bin/reference.bin (one oracle per example dir), which is pre-clipped to [0,BINS) so bucket(v)==v at RUNTIME for that fixture - the modulo is a no-op, so the unconstrained-input strength is only demonstrated at the algorithm-surface/codegen level, not at runtime. A truly-unconstrained-input demonstration needs its OWN example dir (its own input.bin with values outside [0,BINS) + its own reference.bin computed through the modulo bucket) so the bucket() does real work at runtime. Keep separate to avoid perturbing the shared 08 oracle.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 distributed textbook scatter schedule bit-identical across the applicable tier-1 backends (input-index partition; whole-array histogram replicate; host element-wise-sum combine)
- [x] #2 a dedicated example dir ships truly-unconstrained input (values outside [0,BINS)) with a reference.bin computed through the modulo bucket, so bucket() does real runtime work; bit-identical PASS
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implementation Plan (cycle):
AC#1 first, fully gated+committed, before AC#2.

AC#1 distributed textbook scatter:
- New schedule nuc-nucleus/examples/08-histogram/schedules/distributed.textbook.sched.nuc, modeled on distributed.scatter.sched.nuc but `schedule for "../prog.textbook.algo.nuc"`; place bucket+inc on {w0..w3}; loop i partition=workers; transfer input+histogram sync. Honest textbook-true header (no scatter-only facts like "no bucket kernel"; cite TASK-0432).
- 7 [[required]] M6 entries for schedule=distributed.textbook in e2e-matrix.toml with header block.
- Gate: confirm 7 new cells PASS bit-identical vs reference.bin (value-correct, not just non-hanging). Verify cumulative classifier: bucket(input[i]) on both LHS and self-read compares EQUAL => NOT cumulative => stays wrapping_add accumulate fan-in (same class as distributed.scatter). Verify all 7 incl strict-FIFO bufsync/poll.
- Commit.

AC#2 dedicated unconstrained-input fixture:
- New example dir 19-histogram-unconstrained (verify highest number first): prog.textbook.algo.nuc + kernels.textbook.rs (reuse via variant rule), input.bin with values OUTSIDE [0,BINS) incl negatives, reference/ oracle computing through ((v%BINS)+BINS)%BINS (NO strict [0,BINS) validation), reference.bin, README, >=1 schedule.
- Register in runnable_examples + [[required]] entries (M6).
- Document reproducible input.bin generation.
- Gate + commit.

Sequencing: land AC#1 cleanly; if AC#2 ripples, land AC#1 + leave In Progress with honest notes + file prereq if real.

CYCLE OUTCOME (both ACs landed, gated green, value-correct).

AC#1 (commit 9669ec4): 08-histogram/distributed.textbook scatter. New schedule distributed.textbook.sched.nuc (bucket+inc on w0..w3, loop i partition=workers, transfer input+histogram sync) + 7 [[required]] M6 entries. e2e 392/335/0/57/0 -> 399/342/0/57/0 (+7 pass). All 7 tier-1 backends PASS byte-identical vs reference.bin incl both strict-FIFO (bufsync/poll).

AC#2: new example dir 19-histogram-unconstrained. Own input.bin (256 i32, isqrt-skewed bin map offset into unconstrained signed range: 111 negatives, 108 >=BINS, only 37 in-range; post-bucket distribution NON-UNIFORM ramp 1,3,..,31 so the oracle discriminates). Own reference/ oracle uses i32::rem_euclid (structurally != kernel manual modulo, policy §2) and does NOT validate range. reference.bin verified == independently-computed expected histogram. 2 schedules (textbook single-worker + distributed.textbook) x 7 backends = 14 [[required]] M6 cells, all PASS bit-identical. e2e 399/342 -> 413/356/0/57/0 (+14 pass).

CUMULATIVE-CLASSIFIER VERIFICATION (the live risk): confirmed EMPIRICALLY on the distributed transfer-inject path. bucket(input[i]) on both LHS and self-read is the IDENTICAL IrExpr::Call => collect_cumulative_data_names does NOT fire (no structural index difference) => stays wrapping_add ACCUMULATE fan-in (same class as distributed.scatter). All distributed cells byte-correct vs oracle, proving whole-array-replicate + host element-wise-sum is value-correct (not xN-wrong). The bucket() wrapper did NOT trip the classifier.

bucket() DOES REAL RUNTIME WORK in AC#2: 219/256 input values are out-of-range; a bare scatter would index OOB. Byte-match to the rem_euclid oracle proves the compiled bucket()-in-index path evaluates the modulo.

GATE: full `just ci` exited 0 (both negative arms bit correctly: xbackend-corruption 48 applied/15 detected; required-coverage typo detected). Authoritative real e2e 413/356/0/57/0, zero new fails/skips.

LIMITS / scope NOT covered: none beyond ACs. No prereq tasks needed — the classifier and codegen path handled bucket()-in-index distributed cleanly with no new machinery. TASK-0431 (index-arg cast follow-up) unaffected here (bucket takes i32 input[i], returns i32 used as index; the existing as-usize subscript cast path was exercised correctly across all 7 backends).

GOTCHA: first two input.bin generators (linear coprime step, LCG low-bits) produced perfectly UNIFORM post-bucket histograms (16/bin) — a weak oracle that would also pass under a wrong bijective bucket. Switched to an isqrt-skewed map to force a non-uniform ramp distribution so the oracle genuinely discriminates correct bucketing. Documented the generator in the README for reproducibility.
<!-- SECTION:NOTES:END -->
