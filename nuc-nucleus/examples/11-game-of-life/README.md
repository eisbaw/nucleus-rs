# Example 11 — Game of Life (multi-iteration stencil)

The smallest self-contained fixture for the M4 capability set as
applied to an ITERATED stencil: **async transfer + buffer depth > 1 +
notify=event + pipelined loop** with a CROSS-ITERATION data
dependency. One pure kernel (`step`) applied for ITERS generations
over a 1D toroidal grid, with the inter-generation `grid` symbol as
the load-bearing cross-worker edge.

## What this example stresses

| Axis        | What                                                              |
| ----------- | ----------------------------------------------------------------- |
| Algorithmic | Iterated stencil: generation `t+1` reads three cells of generation `t`. Toroidal wrap-around at the spatial boundary via `(i+N-1) % N` and `(i+1) % N`. The iteration axis `t` is exposed at the algorithm level so a schedule can pipeline it. |
| Scheduling  | `naive` (host-only smoke test on every tier-1 backend) and `pipelined` (`loop t : pipeline=2`, `transfer grid : async, buffer=2, notify=event` — `buffer=2` is the literal "double-buffer"). |
| Backends    | `naive` × {pthreads-sync, mp-tcp-bufsync, pthreads-async} all bit-identical. `pipelined` × pthreads-async is the only cell whose capability surface matches; the other two are [[skip]] with the cited capability mismatch. |

This is the M4 cell PRD §9 row 11 calls out: "Game of Life
(multi-iter) — Multi-iteration stencil, double buffer". It is the
companion to example 09 (producer/consumer pipe): both are M4 fixtures
exercising `async + buffer + notify=event + pipeline=D`, but on
different algorithmic shapes — 09 exercises a TWO-STAGE per-sample
chain (stage parallelism across two compute workers), while 11
exercises a SINGLE-STAGE iterated body (state carried across the
iteration axis).

## Algorithm choice (option A, 1D toroidal — not Conway 2D)

PRD §9 row 11 names "Game of Life". Conway's classical Game of Life
is a 2D rule with eight neighbours per cell, with halo regions across
distributed placements. Halo-region synthesis under distributed
placement is TASK-0117 / TASK-0126 territory and exceeds the current
backend's capability. The simpler 1D additive stencil here — three
neighbours per cell, wrap-around at the boundary — is the smallest
concrete realisation of "iterated stencil" that the existing backends
can carry without forcing in unimplemented halo machinery. What is
load-bearing for the M4 fixture is the COMBINATION of:

- A cross-iteration data dependency (generation `t+1` reads
  generation `t`).
- A stencil access pattern (three cells per output cell).
- The buffer=2 + pipeline=2 + notify=event capability surface on the
  inter-iteration data edge.

The cell-update rule itself is the LEAST load-bearing piece — the
SAME M4 surface would be exercised by Conway's eight-neighbour 2D
rule once halo synthesis lands. When that happens, a follow-up
example (or an upgrade to this one) can swap the body to Conway's
true rule without changing the schedules.

## What this example does NOT stress

- **2D stencils with halo regions and distributed placement.**
  TASK-0117 / TASK-0126 territory.
- **Multi-stage pipelining inside the iteration body.** Example 09
  is the two-compute-worker stage-parallelism fixture; this example
  is the iterated-state companion with a single compute kernel.
  Honest scope note in `pipelined.sched.nuc`: with `step` alone on
  `compute`, the per-(t, i) body is a single-worker chunk and
  `transfer_inject`'s same-worker skip (TASK-0214) means no
  per-iteration Xfer is emitted between consecutive `step`
  applications on `compute`. The `loop t : pipeline=2` directive's
  primary effect on this schedule is to drive the IR-level
  `initial_marking=2` on the `grid` Push/Wait pair (so the link gate
  TASK-0134 validates the pipeline-buffer match) and to pin the M4
  capability surface end-to-end — not to introduce genuine
  stage-parallelism inside the iteration loop.
- **Reductions, sorts, or any non-deterministic-under-reorder
  operation.** Each `grid[t+1][i]` depends only on three cells of
  `grid[t]`; ordering across `i` does not affect the output bits.

## I/O format

Binary little-endian `i32` words. `N = 32`; this matches `const N :
usize = 32;` in `prog.algo.nuc`.

- **`input.bin`** (128 bytes):
  - bytes `[0      ..   4*N) ` — array `seed`, `N` LE `i32` words
    (the initial generation, `grid[0]`).
- **`reference.bin`** (128 bytes):
  - bytes `[0      ..   4*N) ` — array `result`, `N` LE `i32` words
    (the final generation after `ITERS=8` applications of `step`).

The committed input pattern is

```
seed[i] = ((i * 5) % 7) + 1
```

i.e. the cyclic sequence `1, 6, 4, 2, 7, 5, 3, 1, 6, 4, 2, 7, 5, 3,
1, ...` — values in `1..=7`. Non-zero (the seed=0 case would zero the
entire grid forever and mask a class of "drop the wrap-around" bugs);
strictly varying across `i` (no constant or alternating period that
the 3-tap sum would smooth into a trivial steady state too quickly);
small enough that 8 generations of 3x growth stay far below `i32::MAX`
(worst-case `7 * 3^8 ≈ 46k`, observed range after 8 iterations is
22373..26894).

Regenerate `input.bin`:

```python
import struct
N = 32
buf = bytearray()
for i in range(N):
    val = ((i * 5) % 7) + 1
    buf += struct.pack('<i', val)
open('input.bin', 'wb').write(buf)
```

### Pinned hashes

```
input.bin     sha256: 51d6bc9ada0a63f847969bcd9aa1691193e416276c424190ee7fba1e81a0871f
reference.bin sha256: f2c2069c773a5cfe7242d3f406ee210dbc9cfd0482f83d8982b18e116ff01a52
```

If `prog.algo.nuc`'s `const N` / `const ITERS`, `kernels.rs`'s `step`,
or the reference implementation's arithmetic ever changes, these
hashes change and the README must be updated in the SAME commit (per
policy §3).

### How to verify

The e2e harness diffs each backend's `output.bin` against
`reference.bin` (TASK-0023). To regenerate and re-verify by hand:

```sh
cargo run --release \
  --manifest-path nuc-nucleus/examples/11-game-of-life/reference/Cargo.toml -- \
  --in    nuc-nucleus/examples/11-game-of-life/input.bin \
  --out   nuc-nucleus/examples/11-game-of-life/reference.bin

sha256sum nuc-nucleus/examples/11-game-of-life/reference.bin
# expect: f2c2069c773a5cfe7242d3f406ee210dbc9cfd0482f83d8982b18e116ff01a52
```

## Reference implementation

`reference/` is a standalone Rust crate with no dependency on
Nucleus, on any backend crate, or on `kernels.rs`. See
[policy §2](../../../docs/reference-impl-policy.md#2-independence)
(the independence rule).

It reads `input.bin`, decodes the seed, runs `ITERS=8` iterations of
the three-tap stencil with toroidal wrap-around — using TWO Vec<i32>
buffers and `std::mem::swap` between them (a true double-buffer in
the everyday sense, distinct from the algorithm's 2D `grid[ITERS+1]
[N]` shape) — and writes the final generation to the output path. No
threads, no third-party crates, no `HashMap` — determinism rule
(policy §5).

The reference deliberately uses a DIFFERENT decomposition of the same
recurrence: where `prog.algo.nuc` exposes the iteration axis as the
OUTER dimension of a 2D `grid` array (so the schedule can pipeline
it), the reference uses two flat buffers and swaps. The per-iteration
step expression (`l + m + r` with `wrapping_add`) is identical
because that IS the recurrence definition; the buffer shape, loop
structure, and boundary handling are independently re-derived. If a
bug drops or reorders the wrap-around indices in the Nucleus emit,
the reference must NOT silently produce the same wrong bytes — the
third-witness argument requires algorithmic similarity, not closed-
form equivalence.

## Numeric type choice: `i32`

Same rationale as every other example. PRD §13 leans toward
integer-only for tier-1 differential testing; integer ops are
bit-deterministic by Rust's language definition. `wrapping_add`
documents the overflow contract; the committed input + ITERS=8 stays
well inside the i32 range (worst-case `7 * 3^8 ≈ 46k`).

## Why `Vec<i32>` and not `[i32; N]` in `kernels.rs`?

Same as examples 01..07/09/13. TASK-0103 is the open PRD question for
aggregate-type matching. Until it lands, aggregate kernel I/O uses
`Vec<i32>` with a runtime length assertion in `save_output`.

## Why the `ident` staging kernels?

The aggregate `load_input : () -> i32[N]` returns a flat `i32[N]`,
but the algorithm's iteration state lives in a 2D `grid :
i32[ITERS+1][N]`. The single-assignment rule (PRD §6.2.1) requires
each `grid[0][i]` to be written exactly once. We bridge the two
shapes with a per-element loop and a scalar `ident` (identity) kernel:

```
seed <-- load_input();
for i : 0 .. N { grid[0][i] <-- ident(seed[i]); }
```

A symmetric loop extracts the final generation into a flat `result`
array for `save_output`. Both `ident` loops compile to scalar moves
under release. The overhead is ~2N kernel calls relative to a
hypothetical direct-sub-array I/O surface, which is the price of
staying inside TASK-0012's scalar contract pass at the current
milestone.

## Contract-check limitation

The contract pass [`check_kernels_contract`](../../../nucleus/nucleus-compiler/src/contract.rs)
(TASK-0012) is scalar-only at present. Running it against this
example produces:

- **PASS** for `step`   — declared `(i32, i32, i32) -> i32`, signature matches.
- **PASS** for `ident`  — declared `(i32) -> i32`, signature matches.
- **`TypeMismatch`** for `load_input`, `save_output` — their Nuc-side
  declarations are aggregate (`i32[N]`) and the current matcher
  emits a loud "aggregate type matching is not yet implemented"
  diagnostic. Loud failure, not silent acceptance; same pattern as
  every other example.

When aggregate matching lands, this example does not need to change.

## Required schedules

| Schedule                  | Backends required at M3/M4                     | Why                                                          |
| ------------------------- | ------------------------------------------- | ------------------------------------------------------------ |
| `naive.sched.nuc`         | `pthreads-sync`, `mp-tcp-bufsync`, `pthreads-async` (M3) | Single-worker smoke test; every tier-1 backend must produce bit-identical output. pthreads-async's single-worker arm delegates to the shared single-worker renderer. |
| `pipelined.sched.nuc`     | `pthreads-async` only (M4 cell)             | Requires `async + buffer=2 + notify=event`; only pthreads-async's capability surface satisfies this. The other two backends are [[skip]] in `e2e-matrix.toml` with the capability mismatch cited verbatim. |

The pipelined schedule places `step` on `compute`, the `ident`
staging on `host`, and runs the inter-generation edge as
`transfer grid : async, buffer=2, notify=event` with
`loop t : pipeline=2`. `buffer=2` exactly matches `pipeline=2` — the
"double-buffer" the PRD row names.
