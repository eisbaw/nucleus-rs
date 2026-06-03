# Example 21 — Jacobi iteration (convergence-check `for..until`)

Landed cycle 262 (epic S5; TASK-0341.02.01.06). The **convergence**
sibling of example 16-jacobi. Same 2D 5-tap Jacobi stencil on an H=W=8
grid with Dirichlet zero boundary, but the loop runs **until** a
data-dependent convergence predicate fires (or a compile-time cap),
instead of a fixed number of iterations.

```
field[t][y][x] = (field[t-1][y-1][x] + field[t-1][y+1][x]
                + field[t-1][y][x-1] + field[t-1][y][x+1]) / 4

maxdiff[t]     = max over the interior of |field[t][y][x] - field[t-1][y][x]|

for t : 0 .. ITERS_CAP+1 until maxdiff[t] <= TOL
```

The seed-staging case `field[0][y][x] = seed[y][x]` lives inside the
same `jacobi5_or_seed` kernel via a branch on `t == 0` (the same
single-Dataflow folding 16-jacobi / 11-game-of-life use). `partials`
and `maxdiff` are indexed by the generation `t` so the per-generation
reduction does not multi-assign a single slot (PRD §6.2.1
single-assignment).

This is the **first non-inert consumer** of the `for..until`
machinery built across epics S1–S4 (grammar surface, RelExpr, bool
lowering, IR + bounded-cap lowering, break emit).

## What this example stresses

| Axis        | What                                                                                |
| ----------- | ----------------------------------------------------------------------------------- |
| Algorithmic | Data-dependent loop termination: a per-generation L-infinity convergence scalar.    |
| Language    | The `for..until COND` bounded early-exit surface (`docs/grammar-algo.md`).           |
| Codegen     | Runtime break-generation final-read + cap-hit observability (single-worker).         |
| Scheduling  | Naive only: every kernel on `host`. No transfers.                                    |
| Backends    | pthreads-sync ONLY ([[required]]); the other six are e2e-skipped (see below).        |

### The `for..until` early-exit

The `..ITERS_CAP+1` upper bound is the **compile-time cap** that keeps
the loop statically bounded (the Petri-net boundedness keystone: an
early-exit prefix `0..k` is a sub-trace of the bounded `0..ITERS_CAP+1`
net, so it is bounded a fortiori — the halt predicate is
analysis-invisible). `until maxdiff[t] <= TOL` is the runtime halt
predicate. The single-worker pthreads-sync backend emits
`if (maxdiff[t] <= TOL) { __nuc_break_gen = t; break; }` as the last
statement of the loop body (epic S4, TASK-0341.02.01.05.04).

### Runtime break-generation final-read (TASK-0341.02.01.05.02)

The extraction reads `field[ITERS_CAP]` at the source level, but on
early-exit at generation `k < ITERS_CAP` that cap slice is **unwritten**
(all-zero). The backend STRUCTURALLY rewrites the extraction's
generation-axis index from the cap to the runtime
`__nuc_final_gen` (the captured break generation `k`), so the converged
generation `field[k]` is read. The step's cross-iteration read index
`(t + ITERS_CAP) % (ITERS_CAP + 1)` is UNCHANGED (it always evaluates
to `t-1` and is break-point-independent).

### Cap-hit-not-converged observability (TASK-0341.02.01.05.03)

If the loop runs the full cap without the predicate ever firing,
`__nuc_break_gen` stays at its `-1` sentinel. The backend emits a
post-loop **stderr** diagnostic (`[[nuc_converge]] did NOT converge
…`) and resolves `__nuc_final_gen` to the cap (the last computed
generation). Cap-hit is therefore OBSERVABLE — not a silent stop that
looks byte-identical to a converged run. The diagnostic is stderr-only,
so the cross-backend differential's stdout / `output.bin` bytes are
untouched (determinism-safe). The committed input converges early, so
this branch is a defensive fallback the fixture does NOT reach.

## Why TOL = 2 (not 0): the runtime final-read must be load-bearing

The early-exit value-correctness is only genuinely tested if the
converged generation `field[k]` DIFFERS from the unwritten cap slice
`field[ITERS_CAP]` (all-zero). With the committed seed `maxdiff[t]`
decays `288, 184, 98, 69, …`; at the **exact** integer fixed point
(`TOL=0`, k=39) the converged interior is **all zeros** — byte-identical
to the unwritten cap slice. A `TOL=0` run would PASS even if the
runtime-final-read rewrite were absent (a silent-drop false pass).

With `TOL=2` the loop breaks at **generation 30** while the interior is
still NON-zero (interior sum 108), so reading the hard-coded
`field[ITERS_CAP]` (zeros) would yield a WRONG result. The runtime read
of `field[k]` is load-bearing and the differential vs `reference.bin`
actually bites. Break gen 30 is comfortably below the cap of 64, so the
early-exit path is exercised.

## What this example does NOT stress

- **Multi-worker / distributed `for..until`.** The break emit is
  tier-1 single-worker pthreads-sync ONLY today; the multi-worker
  walkers (`multi_worker_walker`, `tcp_plan`) fail loud on a
  `break_cond`, and the embedded pattern rejects it. The 7-backend /
  multi-worker break differential is epic **S7 (TASK-0341.02.01.08)**.
  The other six tier-1 backends are `[[skip]]`-ed on this example in
  `e2e-matrix.toml` with that reason; only `naive` × `pthreads-sync`
  is `[[required]]`.
- **Overflow stress in the reduction.** The convergence reduction uses
  the overflow-safe `abs_diff_i32` (i64-widening + `unsigned_abs`,
  TASK-0436) — NOT the S3 fixture's `wrapping_sub().abs()`, which
  panics / mis-ranks at `i32::MIN`. The committed data is well inside
  range, but the kernel is hardened regardless.

## I/O format

Binary little-endian `i32` words. `H = W = 8`, `ITERS_CAP = 64`,
`TOL = 2`. Both arrays are laid out row-major in their declared `[H][W]`
shape.

- **`input.bin`** (256 bytes): bytes `[(y*W + x)*4 .. +4)` —
  `seed[y][x]`, LE `i32`.
- **`reference.bin`** (256 bytes): bytes `[(y*W + x)*4 .. +4)` — the
  CONVERGED generation `field[k][y][x]`, LE `i32`.

The committed `input.bin` seed pattern (interior cells only; the
boundary stays 0 by Dirichlet BC):

```
seed[y][x] = ((y * 13 + x * 7) % 17) * 16 + 32     for 1 <= y < H-1, 1 <= x < W-1
seed[y][x] = 0                                      on the boundary
```

The values vary across both `y` and `x` (so a transposed-loop bug is
observable) and stay in `0..256` (the worst-case 4-tap sum `4*255=1020`
is well inside i32).

## How to regenerate `reference.bin`

Per [policy §1](../../../docs/reference-impl-policy.md#1-file-layout):

```sh
cargo run --release \
  --manifest-path nuc-nucleus/examples/21-jacobi-converge/reference/Cargo.toml -- \
  --in    nuc-nucleus/examples/21-jacobi-converge/input.bin \
  --out   nuc-nucleus/examples/21-jacobi-converge/reference.bin
```

`input.bin` is the committed deterministic seed; regenerate it (only if
the seed pattern changes) with the reference crate's `--gen-input` mode:

```sh
cargo run --release \
  --manifest-path nuc-nucleus/examples/21-jacobi-converge/reference/Cargo.toml -- \
  --gen-input \
  --out   nuc-nucleus/examples/21-jacobi-converge/input.bin
```

## Reference implementation

`reference/` is a standalone Rust crate with no dependency on Nucleus,
on any backend crate, or on `kernels.rs` (policy §2). It reads
`input.bin`, runs the Jacobi step + per-generation L-infinity reduction,
halts at the first `maxdiff[t] <= TOL` (or the cap), and writes the
converged generation `field[k]` LE-encoded row-major. The abs-diff
arithmetic is intentionally identical in spelling to
`kernels.rs::abs_diff_i32` (a semantic mirror, not a code dependency) so
the differential stays meaningful. No threads, no third-party crates,
no HashMap.

## Required schedules

- `naive.sched.nuc` — single worker (`host`), every kernel placed there.
  No loop transforms, no transfers. The only schedule required for this
  example (single-worker `for..until` is the entire S5 scope).
