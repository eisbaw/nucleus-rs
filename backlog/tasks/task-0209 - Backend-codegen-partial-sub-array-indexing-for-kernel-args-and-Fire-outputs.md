---
id: TASK-0209
title: 'Backend codegen: partial sub-array indexing for kernel args and Fire outputs'
status: To Do
assignee: []
created_date: '2026-05-20 20:12'
labels:
  - backend
  - codegen
  - M2
  - blocker
dependencies:
  - TASK-0156
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Both tier-1 backends (pthreads-sync, mp-tcp-bufsync) currently render every
kernel argument and every Fire output as a SCALAR slot access into a flat
Vec<T>. In `nucleus/backends/pthreads-sync/src/lib.rs`:

- `render_flat_index` (line 818) hard-rejects rank-mismatched index lists
  with `EmitError::UnsupportedFeature("rank/shape mismatch with index list")`.
- `render_fire_arg` (line 778) emits `name(flat_idx)` for any indexed
  DataSlice — assumes a single scalar.
- The indexed-assignment branch of `Event::Fire` (line 603) emits
  `name(flat_idx) = kernels::callee(...)` — also single scalar.

Reproducer (verified): example 13 / naive schedule against pthreads-sync.
The compile pipeline reports `nucleus: ok`, but `cargo build` on the
emitted nuc-generated crate fails with E0308 mismatched types: the
generated `kernels::conv_block_1(input(n as usize))` passes a single f32
where the kernel signature expects `Vec<f32>`, because `input` is
declared `f32(B)(C0)(H)(W)` (rank 4) and indexed with just `n` (rank 1).
The output `feat1(n) <-- conv_block_1(input(n))` likewise tries to assign
the kernel's `Vec<f32>` return to a single slot.

Symptom from a stub kernels.rs whose signatures are `(Vec<f32>) -> Vec<f32>`:
  error[E0308]: mismatched types
    -> src/main.rs:14:46
    14   feat1((n) as usize) = kernels::conv_block_1(input((n) as usize));
                                                     ^^^^^^^^^^^^^^^^^^^^
                                                     expected `Vec<f32>`, found `f32`

This blocks every algorithm whose dataflow shape contains partial
indexing into a multi-rank tensor — most notably TASK-0053 (CNN
inference, the entire M6 ML workload). It also blocks any future example
that wants per-sample layer kernels, per-channel kernels, per-row
kernels, etc.

What is needed:
- On the ARGUMENT side: when the DataSlice has FEWER indices than the
  data's rank (partial slice), emit a borrowed sub-slice expression
  whose typed contract matches the kernel parameter. For Vec<T> /
  Box(slice) conventions, the natural Rust spelling is
  `data(start..start+sub_len).to_vec()` (own) or `&data(start..end)`
  (borrowed) depending on the kernel signature convention picked under
  TASK-0103. The stride / sub_len both derive from the existing
  `sidecar.data_type(...).dims` already plumbed by TASK-0150 / 0156.
- On the OUTPUT side: when the indexed-assignment LHS rank < data rank
  (sub-array write), emit a sub-range copy:
    `data(start..start+sub_len).copy_from_slice(&kernels::callee(...))`
- The compiler `contract` pass already reports the aggregate type
  mismatch as a known limitation. Either accept the Vec<T> convention
  end-to-end (and emit the sub-slice idiom above), or close TASK-0103
  with a typed-array convention and emit that.

Determinism: contiguous row-major sub-slice access; no reordering; the
existing examples 01/02/03/04/05/06/07 all use either full-rank scalar
access or whole-array binding (no partial slices), so the new sub-slice
code path is strictly additional and cannot regress them.

Verification:
- pthreads-sync naive build of example 13 emits a generated crate that
  COMPILES (today: E0308 expected Vec<T> found f32).
- A NEW unit test in pthreads-sync renders a synthetic Fire whose
  binding rank < dims and asserts the emitted call uses a sub-slice
  expression, not a single-scalar index.
- Determinism + bit-identical e2e on 01..07 stays green.

Out of scope (separate gating tasks):
- pipeline_parallel for CNN: requires async + buffer=3 + notify=event,
  no tier-1 backend supports those. Belongs in TASK-0117 (distributed
  placement) / a future M5+ async-backend task.
- batch_parallel on mp-tcp-bufsync: host-excluding barrier — already
  filed as TASK-0175.

Why not 'just rewrite CNN to scalar kernels': defeats the example's
pitch (PRD §9 row 13 = layer-wise dataflow; whole-layer kernel
granularity is the load-bearing demonstration of M6's ML workload).
Scalar-conv would be a different example.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Backend codegen renders partial sub-array indexing on the ARGUMENT side as a typed sub-slice (data[start..start+sub_len], with conventional Rust borrowed/owned spelling matching the kernel param type).
- [ ] #2 Backend codegen renders sub-array OUTPUT (LHS rank < data rank) as a sub-range copy_from_slice from the kernel return.
- [ ] #3 pthreads-sync naive emits a nuc-generated crate that cargo-builds for example 13.
- [ ] #4 Unit test in pthreads-sync exercises a rank<dims Fire binding and asserts the sub-slice form is emitted.
- [ ] #5 Determinism + bit-identical e2e on examples 01..07 unchanged.
- [ ] #6 mp-tcp-bufsync naive (or whatever schedule subset is capability-compatible) emits a cargo-buildable crate for example 13.
<!-- AC:END -->
