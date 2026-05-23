# Numeric determinism in v2: integer-only

TASK-0060 / PRD §10.1 / PRD §13. Documents the v2 rule that example
data types are integer (`i8` / `i16` / `i32` / `i64`), and **not**
floating-point. Why: integer arithmetic is bit-deterministic by
language definition; floating-point reductions reorder under parallel
schedules, which breaks the bit-identical differential test.

## The v2 rule

All in-tree example data symbols are integers. The cross-backend
bit-identical differential test (PRD §10.1: "every required cell
produces byte-identical output across every tier-1 backend") is
load-bearing for v2's thesis-falsifiability claim. Any floating-point
data type would risk a parallel reduction reordering bits without
changing the answer to within epsilon — which the bit-identical
oracle correctly flags as a failure but which the user might dismiss
as "numeric noise". v2 sidesteps the question by not exercising the
FP path.

## Verification

`grep -E "f32|f64" nuc-nucleus/examples/*/prog.algo.nuc` returns only
comment lines that explicitly REJECT floating-point — no algorithm
declares an `f32` or `f64` data symbol. The tier-1 examples
(01-elementwise-add, 02-split-add, 03-reduction, 04-prefix-sum,
05-stencil, 06-separable-filter, 07-matmul, 09-producer-consumer,
11-game-of-life, 13-cnn-inference) all use `i32` or `i64` for their
data symbols. Example 14-hearing-aid (M11) will inherit the same rule
when its `kernels.rs` lands.

## Decisions (recorded)

- **Why not epsilon comparison.** v2's e2e gate runs
  `cmp --binary reference.bin output.bin` (byte-equal) at the
  differential phase. An epsilon path would require:
  - per-cell numeric-comparison harness aware of the data type's
    bit layout (f32 IEEE-754 vs f64 vs Q-format);
  - a tolerance budget per algorithm + per schedule (e.g. 1e-6 for
    pointwise ops, larger for accumulators);
  - tests that PROVE the tolerance bites — same negative-arm
    discipline as `determinism-check-negative` (TASK-0188).
  Each is real work; building it for zero FP examples adds machinery
  without test coverage. **Reconsider** if a real FP example needs
  to ship (e.g. v3 DSP demo, or a quantised NN that genuinely needs
  f32 weights).
- **Why not fixed-reduction-order FP.** A fixed-order reduction
  (`fold` in left-to-right source order across all schedules) WOULD
  be bit-identical. But it would also force every schedule to emit
  the same iteration order, eliminating most of the partitioning /
  pipelining freedom the schedule sublanguage exists to expose. The
  bit-identical guarantee in v2 is *across schedules of the same
  algorithm*, not *across reductions of the same data*; fixed-order
  FP would conflate the two.
- **What `13-cnn-inference` does.** The CNN example uses `i32`
  activations + weights (see `kernels.rs` for the chosen ranges that
  stay inside `i32`). It demonstrates the layer-wise dataflow shape
  the cyclic algorithm class supports, not a quantised neural net —
  the "CNN" label is illustrative, not a claim of FP-class
  computation.

## Test for the rule

The rule "no FP in algorithm data declarations" is enforced by
*convention + grep* in v2, not by a compiler check. The cross-backend
bit-identical differential gate (`just e2e`) would surface any FP
non-determinism on a real cell, but only as a runtime cell-failure,
not as an upstream language-level rejection. Adding a parser-level
"FP types require an explicit `tolerance=` annotation" gate is filed
as a follow-up if/when an FP example ships.

## Honest limitations

- The rule is doc + convention, not enforced by the compiler. A
  contributor can add `data x : f32[N]` and the build will accept
  it; only `just e2e` would catch a resulting bit-divergence.
- Integer-only narrows the algorithm class noticeably — no FFT,
  no PDE solvers with real-valued state, no neural-net training.
  This is a deliberate v2 scope choice (PRD §3 non-goals), not a
  bug.
- A future v3 with FP support needs an epsilon-comparator + a
  per-cell tolerance policy + negative-arm tests; this whole doc
  becomes a "see v3-numeric.md" pointer at that point.
