# Example 29 — Jacobi iteration (cap-hit / did-NOT-converge `for..until`)

Landed TASK-0453.05 (rigour epic P5). The **cap-hit** sibling of
example 21-jacobi-converge. Same 2D 5-tap Jacobi stencil on an H=W=8
grid with Dirichlet zero boundary, **same committed seed**, and the
**same** `until maxdiff[t] <= TOL` convergence predicate with `TOL=2` —
but with the compile-time cap deliberately set **below** the
convergence generation so the predicate never fires inside the cap.

```
field[t][y][x] = (field[t-1][y-1][x] + field[t-1][y+1][x]
                + field[t-1][y][x-1] + field[t-1][y][x+1]) / 4

maxdiff[t]     = max over the interior of |field[t][y][x] - field[t-1][y][x]|

for t : 0 .. ITERS_CAP+1 until maxdiff[t] <= TOL     // ITERS_CAP = 16
```

With the committed seed, `maxdiff[t]` first drops to `<= TOL=2` at
generation **30** (verified by example 21-jacobi-converge). This
example sets `ITERS_CAP = 16 < 30`, so the predicate provably never
fires inside the cap: the loop runs the **full `0..ITERS_CAP+1`
worst-case replay**, `__nuc_break_gen` stays at its `-1` sentinel,
`__nuc_final_gen` resolves to `ITERS_CAP`, and the cap-hit stderr
diagnostic fires. The output is `field[16]`, the last computed
generation.

## Why this example exists — the worst-case bound, tested

The bounded `for..until` surface is the static-firing-order-preserving
form of data-dependent iteration: the Petri/soundness gate analyses the
**full-N capped unroll** (the worst case), and any early-exit prefix
`0..k` is a sub-trace of that bounded net (bounded *a fortiori*; the
halt predicate is analysis-invisible). The soundness argument therefore
**rests on the worst-case (full-cap) replay being a real, value-correct
execution path**.

Example 21-jacobi-converge converges at `k=30 < cap`, so it only ever
runs the **converged** branch end-to-end. The cap-hit branch — the
sentinel staying at `-1`, `__nuc_final_gen` resolving to the cap, the
`[[nuc_converge]] did NOT converge ...` diagnostic — was covered **only
by string-level unit tests** (the gap forward-carried from the S5
review, TASK-0341.02.01.06 P3-2). This example closes that gap: it
**runs the full-cap worst case end-to-end** and is byte-identical vs an
independent reference oracle, so the thesis's "bounded data-dependent
iteration: the worst-case replay is bounded by the cap" claim is a
tested reality, not an assertion.

## What this example stresses

| Axis        | What                                                                              |
| ----------- | -------------------------------------------------------------------------------- |
| Algorithmic | Data-dependent loop termination where the predicate does NOT fire (cap-hit).      |
| Language    | The `for..until COND` bounded early-exit surface (`docs/grammar-algo.md`).        |
| Codegen     | Cap-hit observability: `-1` sentinel → `__nuc_final_gen = cap` + stderr (single-worker). |
| Scheduling  | Naive only: every kernel on `host`. No transfers.                                |
| Backends    | pthreads-sync ONLY ([[required]]); the other six are e2e-skipped (S7).            |

### Why the differential bites (avoiding a false pass)

On cap-hit `__nuc_final_gen == ITERS_CAP`, which is exactly the
source-level extraction index `field[ITERS_CAP]`, so the runtime
final-read **rewrite** (TASK-0341.02.01.05.02) is a no-op here — that
rewrite is exercised by 21-jacobi-converge's early exit instead. What
this example's differential bites on is the **control flow**: with
`ITERS_CAP=16` the interior at generation 16 is still mid-diffusion
(NON-zero, and distinct from the seed `field[0]` and from
21-jacobi-converge's `field[30]`), so a compiler that

- broke early when it should not (predicate sense inverted / off-by-one), or
- failed to run the full cap (read an unwritten later slice),

would extract a different generation and diverge from `reference.bin`.

### Stderr-only diagnostic (determinism-safe)

The cap-hit diagnostic is written to **stderr**, so the cross-backend
differential's `stdout` / `output.bin` bytes are untouched. Both the
generated backend and the reference oracle emit the identical
`[[nuc_converge]] did NOT converge within the cap (16 + 1 generations);
extracting the last computed generation 16` line.

## Relationship to 21-jacobi-converge

| | 21-jacobi-converge | 29-jacobi-cap-hit |
| --- | --- | --- |
| `ITERS_CAP` | 64 | 16 |
| `TOL` | 2 | 2 |
| seed (`input.bin`) | shared pattern | **identical** |
| outcome | converges at gen 30 < cap | cap-hit (no convergence in cap) |
| branch exercised | early-exit + runtime final-read rewrite | cap-hit sentinel + did-NOT-converge stderr |
| output | `field[30]` | `field[16]` |

## Cross-backend differential (all 7 tier-1 backends)

This is the **worst-case full-cap replay** witness, and it runs
byte-identically on all 7 tier-1 backends. The `naive` schedule is
host-only (`used_workers <= 1`), so every backend delegates to the SHARED
`render_single_worker_main` (the multi-worker `break_cond` fail-loud guard
is never reached — it fires only for a `partition=workers` schedule). The
`until` predicate NEVER fires inside the cap, so every backend runs the
FULL `0..ITERS_CAP+1` (16 generations), emits the cap-hit stderr
diagnostic (`[[nuc_converge]] did NOT converge ...`), and produces
byte-exact output vs `reference.bin`. So the bounded-data-dependent-
iteration soundness claim ("the worst-case replay is bounded by the cap")
holds bit-identically across all 7 backends. Every backend's cell is
`[[required]]` (epic S7, TASK-0341.02.01.08).

## What this example does NOT demonstrate

- **Multi-worker / distributed `for..until`.** A distributed
  (collective-break) convergence schedule stays out of reach — it
  inherits the 16-jacobi/distributed blockage (honest-BLOCKED on all 7
  backends) plus the new collective all-reduce + broadcast machinery; the
  multi-worker walkers correctly stay fail-loud on `break_cond:Some`
  (pinned by `backend-common/tests/multi_worker_break_cond_rejected.rs`).
  This is the remaining scope of epic S7 (TASK-0341.02.01.08).
- **Floating-point arithmetic.** As 16-jacobi / 21-jacobi-converge,
  integer `/ 4` truncating division is used for bit-determinism.

## Regenerating the fixtures

```
# input.bin (the committed seed — identical pattern to 21-jacobi-converge)
cargo run --release \
  --manifest-path nuc-nucleus/examples/29-jacobi-cap-hit/reference/Cargo.toml -- \
  --gen-input \
  --out   nuc-nucleus/examples/29-jacobi-cap-hit/input.bin

# reference.bin (field[16], the cap-hit last-computed generation)
cargo run --release \
  --manifest-path nuc-nucleus/examples/29-jacobi-cap-hit/reference/Cargo.toml -- \
  --in    nuc-nucleus/examples/29-jacobi-cap-hit/input.bin \
  --out   nuc-nucleus/examples/29-jacobi-cap-hit/reference.bin
```

The reference is standalone (`std` only; no Nucleus dependency) per
`docs/reference-impl-policy.md` §2.
