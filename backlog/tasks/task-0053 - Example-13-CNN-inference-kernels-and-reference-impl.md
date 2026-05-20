---
id: TASK-0053
title: Example 13 (CNN inference) kernels and reference impl
status: In Progress
assignee:
  - '@mped'
created_date: '2026-05-17 23:09'
updated_date: '2026-05-20 20:13'
labels:
  - examples
  - M6
  - validation
dependencies:
  - TASK-0209
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Complete example 13: kernels.rs implementing conv_block_1, conv_block_2, classifier; reference/ Rust impl; input.bin (canned input + canned weights); reference.bin. Required for M6 (full tier-1) and M7 (MPI). Algorithm and schedules already sketched.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 examples/13-cnn-inference/kernels.rs implements all four pure kernels and the two effectful ones.
- [ ] #2 Weights are deterministic — either baked into kernels.rs as const arrays or loaded from a committed binary.
- [ ] #3 examples/13-cnn-inference/reference/ contains an independent reference impl.
- [ ] #4 Required schedules: naive, batch_parallel, pipeline_parallel — all listed in README under M6 are present and reference-matching.
- [ ] #5 Test: all three schedules × all tier-1 backends produce reference-matching output.
- [ ] #6 Implementation notes record design questions (e.g. precision: f32 vs integer scaling for determinism; what fixed-input/fixed-weights mean for the differential test).
- [ ] #7 Implementation notes record honest limitations (no training; small network; no quantisation).
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
<!-- SECTION:NOTES:END -->
