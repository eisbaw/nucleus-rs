---
id: TASK-0022
title: 'Example 3: reduction (sum / min / max)'
status: Done
assignee: []
created_date: '2026-05-17 23:04'
updated_date: '2026-05-18 03:09'
labels:
  - M1
  - examples
  - validation
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Tree-reduction example. Multiple workers compute partial reductions, host combines. Stresses sync barrier semantics and the Barrier SyncKind.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 examples/03-reduction/prog.algo.nuc declares the input and a kernel for the per-element accumulation; the reduction is expressed as a for-loop pattern.
- [ ] #2 examples/03-reduction/schedules/naive.sched.nuc places everything on host (smoke test).
- [ ] #3 examples/03-reduction/kernels.rs implements the accumulator function.
- [ ] #4 examples/03-reduction/reference/ provides the hand-written reference.
- [ ] #5 Test: e2e harness runs this through naive + pthreads-sync; bit-identical output.
- [ ] #6 Implementation notes record design questions (e.g. how to express tree-reduction as a Nuc pattern when v2 has no built-in reduce primitive).
- [ ] #7 Implementation notes record honest limitations (integer reductions only at M1; float reductions reorder and break bit-identity).
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
## Status

Implemented under nuc-nucleus/examples/03-reduction/. All deliverable
files in place; pinning tests landed and green; reference impl is
bit-deterministic across re-runs; the naive-schedule e2e gate passes
bit-identical against reference.bin.

Files added (all under nuc-nucleus/examples/03-reduction/):
- prog.algo.nuc, kernels.rs
- schedules/{naive.sched.nuc, distributed.sched.nuc}
- reference/{Cargo.toml, Cargo.lock, src/main.rs}
- input.bin (1024 B), reference.bin (4 B, decodes to -12520)
- README.md

Pinning tests added (nucleus/compiler/tests/):
- algo_parser.rs::parses_example_03_reduction
- algo_lower.rs::lowers_example_03_reduction
- sched_parser.rs::parses_03_reduction_naive, parses_03_reduction_distributed
- sched_lower.rs::lowers_03_reduction_naive, lowers_03_reduction_distributed
- link.rs::links_03_reduction_naive, links_03_reduction_distributed
- contract.rs::example_03_reduction_contract_passes_scalars_loud_on_load_input
- e2e_example_03.rs::naive_pthreads_sync_bit_identical (passes),
  distributed_pthreads_sync_bit_identical (#[ignore], TASK-0117 + TASK-0126)

## Design questions and choices

### How to express the two-phase reduction in Nuc

The algorithm sublanguage has no built-in reduce primitive and the
single-assignment rule (PRD §6.2.1) applies per data symbol. I read
the rule carefully: a dataflow statement counts as ONE assignment to
the LHS data symbol regardless of whether it sits inside a for-loop
that iterates the indices. That matches the existing example 13
pattern (`feat1[n] <-- conv_block_1(input[n])` inside a `for n` is
recorded as one assignment to `feat1`).

So phase 1 fits as an accumulating dataflow:
  for w : 0 .. NUM_WORKERS {
    for i : 0 .. PARTITION_SIZE {
      partials[w] <-- accumulate(partials[w], a[w][i]);
    }
  }

`partials` is the LHS of exactly one dataflow statement and is
recorded once in the single-assignment table. The codegen
(`collect_pre_init_data` in pthreads-sync) sees `partials` as
indexed-only (no whole-array `<--`) and pre-initialises it to
`vec![0i32; NUM_WORKERS]` — the additive identity for sum. The
generated Rust then is `partials[(w) as usize] =
kernels::accumulate(partials[(w) as usize], a[((w) * 64 + (i)) as usize]);`
which is valid Rust because i32 is Copy: the RHS expression
evaluates first (immutable borrow, returns i32 by copy), then the
LHS mutable borrow opens and writes.

Phase 2 (tree combine) cannot use a loop-fold over a scalar because
the LHS scalar has no index to vary — folding it would be a true
double-assignment. Instead I write the depth-2 fan-in explicitly with
three fresh single-assigned scalars (`half1`, `half2`, `result`).
This is ugly for larger NUM_WORKERS, but NUM_WORKERS=4 is the
load-bearing tier-1 default and the explicit shape is audit-friendly.
A loop-fold over a scratch `i32[NUM_WORKERS]` accumulator would be
the way to scale.

### `partials[w]` as the loop-iteration target

This is exactly what the task brief suggested and it works.
Important detail: the inner loop body uses `partials[w]` on BOTH
sides of the `<--`. The Nuc grammar admits this (it's just an
IndexedLValue on the LHS and an IndexedDataRef on the RHS) and the
lowering passes accept it (the single-assignment table records
`partials` once, not once per iteration).

The codegen handles it via the Vec-of-i32 layout: `Index` for the
read, `IndexMut` for the write. i32 being Copy avoids the borrow
checker complaint.

### Integer arithmetic determinism

PRD §10.1 invariant. Sum is associative AND commutative under
integer addition, so the reference (sequential per-partition) and
the codegen (sequential, also per-partition) produce the same i32
regardless of interpretation order — for the committed fixture, no
overflow happens, but `wrapping_add` makes the contract explicit
(no panic in debug, no UB in release).

### 2D shape for `a` to keep indices identifier-only

I declared `a : i32[NUM_WORKERS][PARTITION_SIZE]` rather than
`i32[N]` for a load-bearing codegen reason: the pthreads-sync
backend's `render_int_expr` (used inside `render_flat_index`) does
NOT resolve const identifiers to their literal values — only
`render_const_expr` (used for loop bounds) does. So writing
`a[w * PARTITION_SIZE + i]` would emit a bare `PARTITION_SIZE`
identifier into the generated Rust where no such const exists,
breaking the build. With the 2D shape, `a[w][i]` rendered via
`render_flat_index` gets the row-major stride from the declared
data shape (a compile-time `usize`) and the iter vars verbatim —
no const lookup needed. Verified by inspecting the generated
src/main.rs: `a[((w) * 64 + (i)) as usize]`. Filed as a follow-up
(TASK-0129) — when the codegen resolves consts in indices, the
shape choice becomes free.

### NUM_WORKERS=4

Matches the task brief and the prospective distributed schedule's
worker count (host + w0..w3). Smallest non-trivial tree depth (=2);
fans in cleanly without a loop-fold.

### Reference impl shape: Cargo project (mirrors examples 01/02)

Same standalone-crate convention as the other examples. Empty
`[workspace]` table to make the workspace exclusion intent
defensive. `panic = "abort"` in release for explicit panic
behaviour.

## Honest limitations

1. **Distributed schedule is shipped as a stretch.** It parses,
   lowers, and links — the `links_03_reduction_distributed` test
   pins that. But `cargo test --test e2e_example_03` with
   `--ignored` would fail: the pthreads-sync backend's
   `distributed_placement_is_rejected` test (TASK-0122) shows
   that `place k on { w0, w1, ... }` is rejected upfront with
   `UnsupportedFeature`. End-to-end depends on
   - **TASK-0117** — iteration-space partitioning for distributed
     placement;
   - **TASK-0126** — per-tile transfer codegen (whole-symbol
     transfers don't suffice for the per-partition slice of `a`
     and per-worker `partials[w]` flow).
   The test is `#[ignore]`'d with a TODO comment naming both.

2. **Sum only.** Min/max would need INT_MIN/INT_MAX as the
   per-`partials[w]` initial value. The pre-init pass defaults to
   zero (the additive identity); v2 has no `init=` clause to override.
   Sum fits cleanly today because its identity matches the default.
   Min/max would require either an explicit init kernel or a
   language change. Filed as **TASK-0130**.

3. **Tree depth fixed at NUM_WORKERS=4.** The depth-2 fan-in is
   written out explicitly (`half1`, `half2`, `result`). For larger
   NUM_WORKERS the right shape is a loop-fold over a scratch
   `i32[NUM_WORKERS]` accumulator, mirroring phase 1's pattern.
   Not implemented because no example needs it yet.

4. **Const-in-index codegen gap.** The 2D shape for `a` is a
   workaround for `render_int_expr` not resolving const
   identifiers. The same issue would bite any algorithm that
   computes flat indices arithmetically from Nuc consts. Filed as
   **TASK-0129**.

5. **Contract pass loud on `load_input`.** Same scalar-only
   limitation as examples 01/02 — `load_input : () -> i32[NUM_WORKERS][PARTITION_SIZE]`
   produces a `TypeMismatch`. The three scalar kernels
   (`accumulate`, `combine`, `save_output`) pass. Pinned by the
   contract test; resolves when TASK-0103 lands.

6. **Single-source-of-truth violation: `const N`** in kernels.rs
   and in reference/src/main.rs both mirror the Nuc-side `const N`
   = 256. Same convention as examples 01/02. Resolves when
   TASK-0103 picks the const-flow convention.

7. **No `notify=`, no `buffer=N`, no async.** Distributed schedule
   uses only `sync` transfers. The pthreads-sync backend's only
   transfer mode, and the right baseline for the M1 invariant.

8. **No `block=`, no `vectorize=`, no `pipeline=`** in either
   schedule. Phase 1 has obvious blocking potential (the
   PARTITION_SIZE inner loop) but blocking is example 5+ territory.

9. **Per-iteration barrier semantics inside loops still oversync.**
   Inherited from TASK-0122 (the multi-worker codegen's coarse
   sync injection). Not exercised by the naive schedule (single
   worker has no Sync nodes), but the distributed schedule when
   it lands would inherit the same over-sync (TASK-0128). Cosmetic
   perf concern, not correctness.

## Verification of acceptance criteria

- **AC #1** prog.algo.nuc declares the input and a kernel for the
  per-element accumulation; the reduction is expressed as a for-loop
  pattern: **MET**. N=256 input, `accumulate(acc, x)` step kernel,
  two nested for-loops in phase 1.
- **AC #2** naive.sched.nuc places everything on host: **MET**.
- **AC #3** kernels.rs implements the accumulator function: **MET**.
  Plus `combine`, `load_input`, `save_output`.
- **AC #4** reference/ provides the hand-written reference: **MET**.
  Standalone Cargo crate, std-only, policy-compliant.
- **AC #5** Test: e2e harness runs this through naive + pthreads-sync;
  bit-identical output: **MET**. `cargo test -p compiler --test
  e2e_example_03` is green; the naive variant is bit-identical
  against reference.bin (the i32 LE value -12520 = 0xFFFFCF18).
- **AC #6** Implementation notes record design questions: **MET**
  (this note).
- **AC #7** Implementation notes record honest limitations (integer
  reductions only at M1; float reductions reorder and break
  bit-identity): **MET** (above; integer only is the standing PRD
  invariant; min/max would need init kernels which v2 doesn't have).

Additional verification beyond AC list:
- algo_parser test: `parses_example_03_reduction` — counts and
  purities asserted (3 consts, 5 data, 4 kernels, 6 top-level
  stmts, nested for-loop shape).
- algo_lower test: `lowers_example_03_reduction` — N=256,
  NUM_WORKERS=4, PARTITION_SIZE=64 resolved; ResolvedType for `a`
  is [4,64]; `result` and `half1`/`half2` are scalar.
- sched_parser/sched_lower tests for both naive and distributed.
- link tests for both schedules (distributed lifts through link
  cleanly).
- contract test pins scalar-pass / load_input-loud behaviour.
- `just check`  → green.
- `just clippy` → green (-D warnings clean).
- `just test`   → green; no regressions; e2e_example_03 has
  1 ignored (distributed stretch).
- `just e2e`    → green (stub harness still).
- Reference impl bit-determinism: re-ran twice; SHA-256
  10fbffbf0983def0806317de1d68aa089017919f578fea61f51f73a857debf14
  both times. cmp clean.
- Inspected generated nucleus/target/e2e-scratch/example_03_naive_pthreads_sync/src/main.rs:
  partials pre-init, nested fold, tree combine, save — all match
  the algorithm.

## Follow-up tasks filed

- **TASK-0129** — pthreads-sync: resolve const identifiers inside
  index expressions (`render_int_expr`/`render_flat_index`). Today
  the codegen leaks bare const identifiers into the generated Rust,
  forcing 2D shape workarounds. When fixed, indices like
  `a[w * PARTITION_SIZE + i]` become free.
- **TASK-0130** — reduction examples with non-zero identities (min,
  max, product). Needs either an explicit init kernel or a
  language-level `init=` clause; v2 today only provides the zero
  identity via pre-init.

Pre-existing tasks that gate the stretch distributed schedule:
- **TASK-0117** — iteration-space partitioning for distributed
  placement.
- **TASK-0126** — per-tile transfer codegen.

Pre-existing tasks that gate the contract-pass cleanups:
- **TASK-0012** follow-ups — aggregate type matching.
- **TASK-0103** — const-in-Rust-generics convention.
- **TASK-0076** — CI gate on reference.bin freshness.
- **TASK-0077** — `just regen-references` recipe.

## Concise summary

Two-phase i32 sum reduction. Phase 1 is a nested for-loop that folds
each partition into `partials[w]` via an accumulating dataflow that
sits cleanly within Nuc's single-assignment-per-symbol rule; the
pre-init pass provides the zero identity. Phase 2 is a depth-2 tree
combine written out explicitly because folding a scalar across a
loop is not expressible. Naive e2e passes bit-identical against the
hand-written reference. Distributed schedule lifts through link but
emit is blocked on per-tile transfer codegen (TASK-0126) and
distributed-placement partitioning (TASK-0117); shipped as a stretch
with `#[ignore]` on the e2e gate.
<!-- SECTION:NOTES:END -->
