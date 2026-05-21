---
id: TASK-0053
title: Example 13 (CNN inference) kernels and reference impl
status: In Progress
assignee:
  - '@mped'
created_date: '2026-05-17 23:09'
updated_date: '2026-05-21 06:28'
labels:
  - examples
  - M6
  - validation
dependencies:
  - TASK-0210
  - TASK-0211
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Complete example 13: kernels.rs implementing conv_block_1, conv_block_2, classifier; reference/ Rust impl; input.bin (canned input + canned weights); reference.bin. Required for M6 (full tier-1) and M7 (MPI). Algorithm and schedules already sketched.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 examples/13-cnn-inference/kernels.rs implements all four pure kernels and the two effectful ones.
- [x] #2 Weights are deterministic — either baked into kernels.rs as const arrays or loaded from a committed binary.
- [x] #3 examples/13-cnn-inference/reference/ contains an independent reference impl.
- [ ] #4 Required schedules: naive, batch_parallel, pipeline_parallel — all listed in README under M6 are present and reference-matching.
- [ ] #5 Test: all three schedules × all tier-1 backends produce reference-matching output.
- [x] #6 Implementation notes record design questions (e.g. precision: f32 vs integer scaling for determinism; what fixed-input/fixed-weights mean for the differential test).
- [x] #7 Implementation notes record honest limitations (no training; small network; no quantisation).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Cycle-1 honest stop (per phase3-ralph cycle-9 honest-stop discipline).
Did NOT land kernels.rs / reference impl / input.bin / reference.bin
this cycle. Reason follows.

CARRIED-OUT INVESTIGATION:
1. Read PRD §10.1 (bit-identical differential), §13 (open questions:
   leaning integer-only for v2), the existing example 01 template
   (kernels.rs + reference/Cargo.toml + reference/src/main.rs + .bin
   layout + README), and docs/reference-impl-policy.md (independence
   rule + determinism rule + audit triple).
2. Read every existing kernels.rs (01..07) to learn the project
   convention. All use Vec<T> for aggregate kernel I/O, and ALL use
   INTEGER types (i32) to dodge f32-reordering non-determinism. The
   CNN example 13's prog.algo.nuc declares f32 — this is a real
   tension (would either need integer rescaling, OR a defined f32
   reduction order, but neither is needed yet because the gap below
   blocks everything earlier).
3. Compiled example 13 with stub kernels (Vec<f32>) against ALL three
   schedules x both tier-1 backends. Result:
     - naive            pthreads-sync   : nucleus reports ok; cargo
                                          build of nuc-generated FAILS
                                          E0308.
     - naive            mp-tcp-bufsync  : nucleus reports ok; cargo
                                          build of nuc-generated FAILS
                                          (same E0308 class).
     - batch_parallel   pthreads-sync   : nucleus ok; cargo build will
                                          fail with same E0308 class.
     - batch_parallel   mp-tcp-bufsync  : nucleus errors loudly with
                                          host-excluding barrier
                                          (TASK-0175) — pre-existing.
     - pipeline_parallel pthreads-sync  : nucleus errors loudly with 12
                                          capability mismatches (async +
                                          buffer=3 + notify=event NOT
                                          supported by any tier-1
                                          backend).
     - pipeline_parallel mp-tcp-bufsync : same capability mismatches.

ROOT CAUSE of the naive-schedule E0308:
The backend codegen path renders every Fire argument and every
indexed-output assignment as a SCALAR slot access into the flat Vec<T>
target. Specifically, in nucleus/backends/pthreads-sync/src/lib.rs:
  - render_flat_index (line 818) and render_fire_arg (line 778) treat
    s.indices.len() == data.dims.len() as the only supported case;
    partial indexing (fewer indices than dims) returns
    EmitError::UnsupportedFeature.
  - The indexed-assignment branch of Event::Fire (line 603) emits
    `data[flat_idx] = kernels::callee(...)` — single scalar write.

The CNN example's dataflow `feat1[n] <-- conv_block_1(input[n])` has
input rank 4 indexed with 1, output rank 4 written with 1 index —
EVERY firing is partial-rank on BOTH sides. The current codegen
hard-fails this case.

This is NOT a kernels.rs bug. It is NOT a reference.bin bug. It is a
backend codegen feature gap. No amount of kernel-body / reference-impl
work in this task can produce a bit-identical e2e cell while the
generated nuc-generated crate fails to compile.

FILED FOLLOW-UPS:
  - TASK-0209: backend codegen support for partial sub-array indexing
    (kernel args + Fire outputs). BLOCKER for TASK-0053. Depends on
    TASK-0156 (DONE). Has 6 acceptance criteria including a synthetic
    unit test and example 13 naive emitting a cargo-buildable crate.
  - TASK-0210: pipeline_parallel deferred until a tier-2 async + buffer
    + event-capable backend lands; do NOT promote to required on any
    tier-1 backend in the interim.
  - batch_parallel on mp-tcp-bufsync is already filed as TASK-0175
    (host-excluding barrier).

CYCLE-1 SCOPE THAT LANDED:
  None of TASK-0053's acceptance criteria (#1-#7) was ticked this
  cycle. The investigation above is the deliverable, plus TASK-0209
  and TASK-0210 filed precisely. TASK-0053 stays IN PROGRESS.

VERIFICATION GATE NOT RUN (no code changes):
  baseline `just test` was sampled at 468 passed / 0 failed / 2 ignored
  (matches the orchestrator's preflight target).

NEXT CYCLE PRECONDITION:
  TASK-0209 must close before TASK-0053 can land naive bit-identical.
  Once TASK-0209 lands, the CNN kernels.rs design question (f32
  determinism strategy) becomes the next decision — likely either:
    (a) fixed-Q integer rescaling end-to-end (matches existing
        integer-only examples; the algorithm declares i32 not f32 —
        but that changes prog.algo.nuc, which is a semantic algorithm
        change per docs/reference-impl-policy.md §3); OR
    (b) strict left-to-right f32 reduction in BOTH kernels.rs AND
        reference/ with a stated reduction order (docs/reference-impl-
        policy.md §5 second bullet permits this).
  Will be decided when TASK-0053 resumes.

Cycle-2 LANDED (post TASK-0209). Single commit on master.

ACs status (honest):
- [x] #1 kernels.rs implements all four pure (conv_block_1,
      conv_block_2, classifier) + two effectful (load_input,
      save_output) kernels.
- [x] #2 Weights deterministic (integer formulae keyed by index in
      BOTH kernels.rs and reference/src/main.rs); inputs from
      committed input.bin.
- [x] #3 reference/ is an independent std-only Rust crate per
      docs/reference-impl-policy.md §2 (no nucleus deps, no shared
      code with kernels.rs, [workspace] empty so it stays outside the
      compiler workspace). Same algorithm, separate source; control
      structure expressed differently (generic forward_conv_pool vs
      two explicit conv1/conv2 bodies).
- [ ] #4 NOT FULLY TICKED. Only `naive` is reference-matching this
      cycle. `batch_parallel` blocked by TASK-0211 (new follow-up:
      multi-worker transfer-distribution gap); `pipeline_parallel`
      blocked by TASK-0210 (tier-2 async+buffer+event capability).
      Per cycle-2 task brief, batch_parallel and pipeline_parallel
      are deferred — they are NOT regressions, they are tracked
      capability/codegen gaps with precise follow-ups filed.
- [ ] #5 NOT FULLY TICKED. Naive cells on both tier-1 backends are
      byte-identical to reference.bin (PASS in e2e-matrix.toml as
      REQUIRED). batch_parallel and pipeline_parallel are SKIPPED
      with task pointers in [[skip]] entries — informational, not
      required-fail.
- [x] #6 README §"Numeric type choice: i32" + §"Weights" + §"I/O
      format" record the design questions (precision, weight
      determinism, file layout, regeneration commands, classifier
      modulus rationale). prog.algo.nuc comment header documents the
      i32 choice with verbatim PRD §13 citation.
- [x] #7 README §"Honest limitations" records: no training, tiny
      network, no quantisation, no bias, no softmax, hand-crafted
      weights, only naive differentially green this cycle.

Numeric choice (verbatim PRD citation, §13 "Bit-identical output
across backends"):
  "Trivial for integer algorithms; non-trivial once floating-point
   reductions enter (sum order matters). Either restrict examples to
   integer/deterministic FP, or compare with epsilon. Leaning toward
   integer-only for v2."
This example follows that lean: prog.algo.nuc declares i32; kernels.rs
and reference/ are i32 throughout; every reduction is a strict
left-to-right for loop using i32::wrapping_mul / i32::wrapping_add.

Verification gate (7-step, all green):
1. just test                  : 469/0/2 (baseline maintained)
2. cargo clippy               : clean -D warnings
3. just e2e                   : 36 / 28 PASS / 0 FAIL / 8 SKIPPED /
                                0 required-fail
                                (pre-cycle 30/26/0/4/0 -> delta +6
                                 cells / +2 PASS / +4 SKIP)
4. just determinism-check (x2): byte-identical across both runs
5. just determinism-check-negative : NUC_NONDET_PERTURBED_CELLS=28;
                                     bites correctly
6. just xbackend-check-negative    : NUC_XBACKEND_CORRUPTED_DETECTED=1;
                                     bites correctly
7. just ci                    : exit 0

Per-cell matrix changes:
- runnable_examples += "13-cnn-inference"
- [[required]] 13-cnn-inference / naive / pthreads-sync (M3)
- [[required]] 13-cnn-inference / naive / mp-tcp-bufsync (M3)
- [[skip]]     13-cnn-inference / batch_parallel / pthreads-sync
                                  (reason: TASK-0211 multi-worker
                                  transfer-distribution gap)
- [[skip]]     13-cnn-inference / batch_parallel / mp-tcp-bufsync
                                  (reason: TASK-0175 + TASK-0211)
- [[skip]]     13-cnn-inference / pipeline_parallel / pthreads-sync
                                  (reason: TASK-0210 async+event)
- [[skip]]     13-cnn-inference / pipeline_parallel / mp-tcp-bufsync
                                  (reason: TASK-0210 async+event)

Test fixups (algo type change ripple):
- compiler/tests/algo_lower.rs::lowers_example_13_cnn_inference: feat1
  ScalarType assertion F32 -> I32.
- backends/pthreads-sync/tests/emit.rs::partial_index_lowers_to_sub_slice:
  scratch stub kernels.rs now uses Vec<i32>. Substring assertions on
  the emitted main.rs are unchanged.

Follow-ups filed:
- TASK-0211 (NEW): multi-worker transfer-distribution gap. Replaces the
  cycle-1 placeholder. Has 5 ACs covering uniform-recv emission, e2e
  build green, synthetic unit test, matrix promotion, and 01..07
  no-regression.
- TASK-0210 (PRE-EXISTING): pipeline_parallel tier-2 capability gap.

Honest limits / not done this cycle:
- batch_parallel / pipeline_parallel both deferred per cycle-2 brief
  scope; precise reasons in the [[skip]] entries.
- Cross-architecture i32 determinism is by Rust language definition;
  not separately exercised here (no foreign-arch CI runner). i32
  wrapping_mul/wrapping_add is bit-deterministic on any supported
  target.
- The "CNN" is a shape demo, not a real neural network. No
  training, no quantisation, no calibration. Weights are deterministic
  integer formulae chosen for uniqueness across classes and bounded
  i32 accumulators, NOT for any classification accuracy.
- Cargo.lock for the reference crate IS committed (matches the
  convention of all other examples/01..07).

## TASK-0117 cycle-1 partial unblock (claude, 2026-05-21)

TASK-0117 cycle-1 landed transfer-injection fan-out + sync-injection co-fix; example 13 batch_parallel × pthreads-sync now cargo-builds AND is byte-identical to reference.bin (sha256 d893337208d7b469…). The cell is [[required]] in nuc-nucleus/e2e-matrix.toml.

### Updated AC status

- AC#4 (Required schedules: naive, batch_parallel, pipeline_parallel — all listed in README under M6 are present and reference-matching): PARTIAL.
  - naive × {pthreads-sync, mp-tcp-bufsync}: GREEN (was already so).
  - batch_parallel × pthreads-sync: GREEN (new, this cycle).
  - batch_parallel × mp-tcp-bufsync: blocked on TASK-0175 (host-excluding barrier) — see e2e-matrix.toml skip reason.
  - pipeline_parallel × both backends: blocked on TASK-0210 (async + buffer=3 + notify=event capability).

- AC#5 (all three schedules × all tier-1 backends produce reference-matching output): PARTIAL. 3 of 6 cells now green; 3 still blocked behind TASK-0175 / TASK-0210.

TASK-0053 stays In Progress; the remaining gaps (mp-tcp host-excluding barrier; pipeline_parallel capability) are tracked separately.
<!-- SECTION:NOTES:END -->
