---
id: TASK-0209
title: 'Backend codegen: partial sub-array indexing for kernel args and Fire outputs'
status: Done
assignee:
  - '@claude'
created_date: '2026-05-20 20:12'
updated_date: '2026-05-20 20:44'
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
- [x] #1 Backend codegen renders partial sub-array indexing on the ARGUMENT side as a typed sub-slice (data[start..start+sub_len], with conventional Rust borrowed/owned spelling matching the kernel param type).
- [x] #2 Backend codegen renders sub-array OUTPUT (LHS rank < data rank) as a sub-range copy_from_slice from the kernel return.
- [x] #3 pthreads-sync naive emits a nuc-generated crate that cargo-builds for example 13.
- [x] #4 Unit test in pthreads-sync exercises a rank<dims Fire binding and asserts the sub-slice form is emitted.
- [x] #5 Determinism + bit-identical e2e on examples 01..07 unchanged.
- [x] #6 mp-tcp-bufsync naive (or whatever schedule subset is capability-compatible) emits a cargo-buildable crate for example 13.
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
TASK-0209 LANDED (cycle-1, single commit).

Implementation summary:
- New private helper classify_data_slice(DataSlice, ctx) -> SliceForm
  in pthreads-sync/src/lib.rs. Classifies an indexed DataSlice into:
    * SliceForm::Scalar(idx_expr) for full-rank access (indices.len()==dims.len()).
    * SliceForm::SubArray { start, sub_len } for partial-prefix access
      (indices.len() < dims.len()). sub_len = product(dims[indices.len()..]).
- New private helper render_fire_output_assign(o, rhs, ctx). Emits either
    name[idx] = rhs;                              (Scalar)
  or
    name[start..start + sub_len usize].copy_from_slice(&rhs); (SubArray)
- New pub shim render_fire_output_assign_pub for the mp-tcp-bufsync and
  pthreads-sync-multi-worker call sites. ONE impl across all three
  Fire-output sites -> no codegen drift across backends.
- render_fire_arg gains sub-array branch: emits
    name[start..start + sub_len usize].to_vec()
  for partial-prefix arg access (owned Vec<T> matches rust_type_of's
  aggregate kernel-param spelling).

Backend touch points (all 3 call sites use the same helper):
- nucleus/backends/pthreads-sync/src/lib.rs:603 (single-worker Fire)
- nucleus/backends/pthreads-sync/src/multi_worker.rs:463 (multi-worker pthreads)
- nucleus/backends/mp-tcp-bufsync/src/lib.rs:632 (mp-tcp Fire)

Byte-identical determinism for examples 01..07 preserved:
- 1D-data scalar access (out[i] on i32[N]) still emits ({i0}) as usize via
  the special-cased indices.len()==1 dims.len()==1 path.
- Multi-dim full-rank scalar access still emits the same
  ((i0)*D1 + (i1)) as usize sum from classify_data_slice's multi-dim path
  (terms vec).
- Determinism gate ran twice (just determinism-check): 26/30 PASS,
  identical to baseline.

Example 13 cargo-buildability proven (AC#3, AC#6):
- pthreads-sync naive emit + cargo build of the emitted crate: OK with
  a stub kernels.rs.
- mp-tcp-bufsync naive emit + cargo build: OK. Emitted main body is
  byte-identical to pthreads-sync's single-worker emission for this
  example, confirming the shared-renderer guarantee.

Gate (full 7-step, all green):
1. just test         : 469 passed / 0 failed / 2 ignored (baseline 468 + 1 new test)
2. clippy            : clean -D warnings
3. just e2e          : 30 / 26 PASS / 0 FAIL / 4 SKIPPED / 0 required-fail (baseline)
4. determinism-check : 30/26 PASS x 2 (byte-identical)
5. determinism-check-negative : bites (NUC_NONDET_PERTURBED_CELLS=26, 26 cells failed as expected)
6. xbackend-check-negative    : bites (NUC_XBACKEND_CORRUPTED_DETECTED=1)
7. just ci           : exit 0

Honest limits:
- Did NOT add example 13 to the e2e matrix or commit a real kernels.rs;
  that is TASK-0053's scope. The unit test inlines a stub kernels.rs
  into a scratch dir and asserts cargo check passes on the emitted
  crate, which is enough to prove the codegen contract.
- Did NOT use NUC_TRACE; no diagnostic emission was needed.
- Did NOT delete the now-zero-caller `render_flat_index` / `render_flat_index_pub`
  pub API. Keeping them as a deprecation surface (next backend wanting
  scalar-only flat indexing has it); removal is cosmetic and would
  rename the workspace API. Filable as a follow-up.
- Did NOT touch the cargo-fmt drift across other workspace files
  (compiler/, e2e/, drivers/, etc.) that the repo HEAD already had
  pre-existing. That is not TASK-0209's scope; filable as a workspace
  hygiene follow-up.
- Did NOT validate non-prefix partial indexing (e.g. fix inner dim,
  leave outer free). classify_data_slice's contract is prefix-rank
  only; the AlgoIR surface syntax cannot produce a non-prefix slice
  (D[a][b] always indexes outer-first), so this is a contract floor
  not a user-visible limitation. If a future surface gains
  D[*][k]-style emission, classify_data_slice would need extension
  AND a non-contiguous gather codegen path.

Cross-backend determinism caveat: contiguous row-major sub-slice
access; no reordering; same impl on both backends -> bit-identical
emission verified by diff on example 13.
<!-- SECTION:NOTES:END -->
