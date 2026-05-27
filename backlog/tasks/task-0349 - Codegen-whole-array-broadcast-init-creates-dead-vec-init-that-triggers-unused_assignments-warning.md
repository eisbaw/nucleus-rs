---
id: TASK-0349
title: >-
  Codegen: whole-array broadcast init creates dead vec! init that triggers
  unused_assignments warning
status: To Do
assignee: []
created_date: '2026-05-27 18:03'
labels:
  - codegen
  - cosmetic
  - multi-worker
  - quality
dependencies: []
priority: low
---

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
=== Filed cycle 212 (TASK-0341.03.02 codegen-warning surface) ===

## Problem

The multi-worker codegen emits, per worker, for each broadcast (whole-array, NOT slice-paste) data symbol:

  let mut x: Vec<i32> = vec![0; 8];
  ...
  x = w0_slot_8.wait();   // recv `x` from host

The initial `vec![0; 8]` is fully overwritten by the assignment, so Rust emits `warning: value assigned to `x` is never read; help: maybe it is overwritten before being read?`. Observed 4 times in the cycle-212 17-spmv/distributed × pthreads-sync emit (one per worker).

## Why it works correctly (despite the warning)

The output is bit-identical against reference.bin on every tier-1 backend — the runtime semantics are correct. The wasted allocation is `vec![0; N]` (a single Vec<i32> of length N=8 in 17-spmv) per worker, per broadcast data: bounded and small. Not a correctness defect.

## Root cause sketch

Compare slice-paste data (e.g. `val` in 17-spmv/distributed):

  let mut val: Vec<i32> = vec![0; 24];
  ...
  { let _tmp = w0_slot_4.wait(); val[0..6].copy_from_slice(&_tmp[0..6]); }  // recv slice

The slice-paste path keeps the init bytes for the OUT-OF-RANGE slots (val[6..24] stays 0 on w0). Those out-of-range slots are then READ by the cargo compiler so the init is live.

The broadcast path overwrites the WHOLE vec, so the init is dead. Two options at the codegen layer:
- (a) Emit `let x: Vec<i32> = w0_slot_8.wait();` directly (no preallocation; the assignment is also the declaration).
- (b) Emit `let mut x: Vec<i32> = Vec::with_capacity(8); x = w0_slot_8.wait();` — saves the zero-init but still has a dead alloc.

Option (a) is cleanest. It requires distinguishing whole-array recv from slice-paste recv at codegen time — the data is already in the codegen layer (the leading-axis filter at TASK-0301 emits empty bounds for whole-array, populated bounds for slice-paste).

## Why this is NOT urgent

- Cosmetic only — output is bit-identical, no runtime cost beyond a transient vec! that the optimizer likely elides at -O3.
- 4 warnings per multi-worker emit is below noise threshold; it does not fail clippy (the warnings are on the EMITTED code, not the workspace code).
- Memory project-negative-seam-and-backend-layout — the codegen output is what users see when they emit + cargo build a nucleus project, so this is user-facing cosmetic warning noise eventually worth removing.

## Acceptance criteria (when picked up)

1. The codegen distinguishes whole-array recv from slice-paste recv at emit time.
2. The emitted code uses `let x = slot.wait();` for whole-array recv (no preallocation).
3. The slice-paste path is unchanged.
4. Cargo build of emitted projects (across all 7 tier-1 backends) emits NO `unused_assignments` warnings on the broadcast-recv pattern.
5. e2e remains bit-identical (no behavioral change).

## Companion / linkage

- Surfaced by TASK-0341.03.02 cycle 212 (17-spmv/distributed); the pattern exists in EVERY multi-worker distributed schedule with a whole-array-broadcast data (07-matmul `b`, 16-jacobi `seed` if cleanly broadcast, 05-stencil's `img_in` under leading_axis_slice, etc.). The cosmetic-only nature means this is LOW priority but the audience for "emitted code looks professional" is real.
- Related: project-negative-seam-and-backend-layout (each backend's emitted main.rs surface).
<!-- SECTION:NOTES:END -->
