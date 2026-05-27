# Example 16 — Jacobi iteration (fixed N)

Landed cycle 206 (TASK-0341.02 AC#1 language-sanity slice). Two-
dimensional Jacobi 4-tap stencil applied repeatedly to a fixed
number of iterations (ITERS=4) on an H=W=8 grid with Dirichlet zero
boundary.

```
field[t][y][x] = (field[t-1][y-1][x] + field[t-1][y+1][x]
                + field[t-1][y][x-1] + field[t-1][y][x+1]) / 4
```

for `t in 1..=ITERS`, `y in 1..H-1`, `x in 1..W-1`. The seed-staging
case `field[0][y][x] = seed[y][x]` lives inside the same kernel via a
branch on `t == 0` (same single-Dataflow folding pattern as
11-game-of-life). In `prog.algo.nuc` the loop walks `t : 0..ITERS+1`
(i.e. `[0, ITERS]` inclusive) so the seed-staging slice and the
ITERS iteration slices are written by one Dataflow; the
"`t in 1..=ITERS`" range above is the human-readable iteration
semantics, not the for-loop bound.

## What this example stresses

| Axis        | What                                                                       |
| ----------- | -------------------------------------------------------------------------- |
| Algorithmic | 2D + multi-iteration: each generation reads cells of the previous one.     |
| Scheduling  | Naive only at AC#1: every kernel on `host`. No transfers, no blocking.     |
| Backends    | pthreads-sync only at AC#1 (formal); informationally PASS on the others.  |

The combination "2D stencil access + cross-iteration read-after-write
on the same data symbol" is exactly the PRD §9 "iterated stencil"
shape — 11-game-of-life is the 1D circular variant; this is the 2D
bounded-grid Jacobi variant. Both rely on the same single-Dataflow
trick: ONE statement on the `field` symbol covers all `ITERS+1`
generations + the seed case, kernel branches on `t`.

## What this example does NOT stress (yet)

- **Convergence-check / data-dependent termination** (TASK-0341.02
  AC#2). The Nuc grammar has no `if`, no `while`, no `break`; loop
  bounds must be compile-time const expressions. A "run until
  max-abs-diff < tolerance" variant is structurally inexpressible
  today and is the AC#2 gap-probe outcome filed as an honest-BLOCKED
  follow-up — naming the missing primitive (data-dependent loop
  termination).
- **Multi-worker distributed schedules.** Filed as a follow-up; same
  precedent as 15-transpose's AC#2 (TASK-0341.01.01). The 4-neighbour
  stencil should reuse `halo_inference` machinery the same way
  05-stencil already does, but verifying that against a 2D
  multi-iteration shape is its own cycle.
- **Floating-point arithmetic.** Jacobi's natural `/ 4.0` average is
  order-of-summation sensitive under parallel reduction. We use
  integer `/ 4` (truncating); the precision hit is the price of
  bit-determinism across schedules and backends — same trade as
  05-stencil's `sum / 9` box blur.

## I/O format

Binary little-endian `i32` words. `H = W = 8`, `ITERS = 4`. Both
arrays are laid out row-major in their declared `[H][W]` shape.

- **`input.bin`** (256 bytes):
  - bytes `[(y*W + x)*4 .. +4)` — `seed[y][x]`, LE `i32`.
- **`reference.bin`** (256 bytes):
  - bytes `[(y*W + x)*4 .. +4)` — `field[ITERS][y][x]`, LE `i32`.

Each fixture is 256 bytes, well under the 10 KB cap that keeps them
inspectable by hand (`hexdump -C input.bin | less`).

The pattern used in `input.bin`:

```
seed[y][x] = (y * 17 + x * 13) & 0xFF
```

These values vary across both `y` and `x` (so a transposed-loop bug
is observable). The `& 0xFF` caps every word at 0..256: the worst-case
4-tap sum is `4 * 255 = 1020`, well inside the i32 range, and `/ 4`
truncates to 0..256 each iteration. Boundary cells stay 0 throughout
(Dirichlet zero BC by single-assignment default).

## How to regenerate `reference.bin`

Per [policy §1](../../../docs/reference-impl-policy.md#1-file-layout):

```sh
cargo run --release \
  --manifest-path nuc-nucleus/examples/16-jacobi/reference/Cargo.toml -- \
  --in    nuc-nucleus/examples/16-jacobi/input.bin \
  --out   nuc-nucleus/examples/16-jacobi/reference.bin
```

`input.bin` is regenerated only if the I/O format changes. The
generator is short enough to inline here for auditability:

```python
import struct
H, W = 8, 8
buf = bytearray()
for y in range(H):
    for x in range(W):
        val = (y * 17 + x * 13) & 0xFF
        buf += struct.pack('<i', val)
open('input.bin', 'wb').write(buf)
```

## Reference implementation

`reference/` is a standalone Rust crate with no dependency on
Nucleus, on any backend crate, or on `kernels.rs`. See policy §2.

It reads `input.bin`, decodes the `[H][W]` seed, allocates
`field[0..=ITERS][H][W]`, copies the seed into `field[0]`'s interior
(boundary stays 0), then runs ITERS Jacobi steps with the same
arithmetic the algorithm uses (`wrapping_add` for the 4-tap sum,
truncating `/ 4`), and writes `field[ITERS]` LE-encoded row-major. No
threads, no third-party crates, no HashMap.

## Single-assignment shape (PRD §6.2.1)

Three top-level `data` symbols, each the LHS of exactly one Dataflow
statement at the statement level:

- `seed` — assigned once by `seed <-- load_input();`.
- `field` — assigned ONCE at the for-nest body level
  (`field[t][y][x] <-- jacobi5_or_seed(...);`). The `(t, y, x)`
  iteration domain covers `t in 0..ITERS+1`, `y in 1..H-1`,
  `x in 1..W-1`; each interior cell of each generation is written
  exactly once. Boundary cells of every generation stay at their
  single-assignment default of 0.
- `result` — assigned once via the `ident` copy nest over the FULL
  grid (boundary + interior) of `field[ITERS]`.

The kernel-side branch on `t` is what allows ONE Dataflow stmt to
cover both the seed-staging case (`t == 0`) and the iteration case
(`t >= 1`). Removing the branch would force TWO Dataflows on `field`,
which the algo-lowering pass rejects as `DoubleAssignment` (PRD
§6.2.1 single-assignment).

## Why integer division (and not float average)

PRD §10.1 and PRD §13 demand bit-deterministic output across
schedules and backends. Rust's `i32 / i32` is truncating integer
division — deterministic, no rounding mode, no platform variation.
A "true average" via float would be order-of-summation sensitive and
reorderable (which is precisely what schedules will reorder under
multi-worker partitioning). We take the precision hit. The reference
impl uses the SAME expression so the differential test stays
meaningful.

The arithmetic flow is:
1. 4-tap sum via three `wrapping_add` calls (documents the overflow
   contract; the committed fixture never wraps in practice).
2. `/ 4` truncating integer divide.

`/ 4` is equivalent to `>> 2` on positive values; we use `/` to make
the semantics explicit at the type level.

## Why `Vec<i32>` and not `[i32; H*W]`?

Per TASK-0103 (Done cycle 17), `Vec<i32>` + runtime length check is
the canonical convention for aggregate-typed kernel signatures. The
algorithm declares `seed : i32[H][W]` and `result : i32[H][W]`; on the
Rust side both are flat `Vec<i32>` of length H*W=64. The internal
`field` symbol does not cross the kernel-IO boundary — only `seed`
(in via `load_input`) and `result` (out via `save_output`) need a
Rust aggregate spelling.

## Contract-check limitation

The contract pass `check_kernels_contract` (TASK-0012) is scalar-only
at present:

- **PASS** for `jacobi5_or_seed` — six scalar i32 params, scalar i32
  return.
- **PASS** for `ident` — `(i32) -> i32`.
- **`TypeMismatch`** for `load_input`, `save_output` — declared
  aggregate (`i32[H][W]`) and the current matcher emits a loud
  "aggregate type matching is not yet implemented" diagnostic.
  Intended behaviour at TASK-0012's scope, not a bug.

## Required schedules

- `naive.sched.nuc` — single worker (`host`), every kernel placed
  there. No loop transforms, no transfers. The only schedule required
  for AC#1 conformance.
