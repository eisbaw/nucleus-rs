# Example 05 — 3x3 box-blur stencil

A small 3x3 box-blur over an `i32` image. For each interior pixel
`(y, x)`, output the truncating integer mean of the nine surrounding
input pixels. Boundary pixels (the outermost ring) are left at zero
in the output — matching the algorithm's single-assignment pattern,
which writes only the interior.

This is the PRD §9 row 5 example: the canonical halo-region /
blocking / reuse stress.

## What this example stresses

| Axis        | What                                                                  |
| ----------- | --------------------------------------------------------------------- |
| Algorithmic | Nine-neighbour access pattern (`img_in[y-1..y+1][x-1..x+1]`) — the first example where the compiler has to read kernel arguments to infer a *halo region* for distributed schedules. |
| Scheduling  | `block=` loop tiling on the outer axis (TASK-0030). The structural rewrite to a (tile-loop, intra-tile-loop) nest is in place; per-tile transfer hoisting is TASK-0143 (open). |
| Backends    | Naive cell: bit-identical against `reference.bin` under `pthreads-sync`. Blocked cell: `#[ignore]`'d on the e2e gate pending TASK-0142 (remainder tiles) + TASK-0143 (per-tile transfers). |

This is the v2 example. The pre-existing `prog.algo.nuc` used the
legacy 2013-style `kernel NAME(...) -> out where pure {{ ${out} =
... }}` substitution syntax, which the grammar (`docs/grammar-algo.md`
§4.3) explicitly retires. TASK-0078 covered the rewrite; this
README reflects the v2 surface.

## What this example does NOT stress

- Distributed placement of `blur3` across compute workers
  (`place blur3 on { w0, w1, w2, w3 }` with halo synthesis). The
  schedule file `schedules/distributed.sched.nuc` declares one such
  schedule for parser/lowering coverage, but the e2e gate is blocked
  on TASK-0117 (iteration-space partitioning for distributed
  placement) and on halo synthesis on top of that.
- `reuse` (sliding-window 3-wide recycle). Listed in
  `distributed.sched.nuc` as `loop x : block=64, vectorize=8, reuse;`
  for parse-coverage; the `reuse` semantics is not yet wired
  end-to-end.
- Multi-iteration stencils. That's example 11 (Game of Life).
- Floating-point arithmetic. PRD §10.1: integer for bit-identity.

## Files

```
05-stencil/
  prog.algo.nuc                 # v2 algorithm (signature-only kernels)
  kernels.rs                    # Rust bodies (i32 blur, file-based I/O)
  schedules/
    naive.sched.nuc             # single-worker smoke test (gate-passing)
    blocked.sched.nuc           # loop y : block=4; single worker (e2e #[ignore]'d, see below)
    distributed.sched.nuc       # four-worker stretch (parser/lower only)
  reference/                    # hand-written, std-only reference impl
    Cargo.toml
    src/main.rs
  input.bin                     # 1024 bytes — 16*16 i32 LE
  reference.bin                 # 1024 bytes — expected output
```

## I/O format

Binary little-endian `i32` words. `H = 16`, `W = 16`; matches `const
H : usize = 16;` and `const W : usize = 16;` in `prog.algo.nuc`.

- **`input.bin`** (1024 bytes):
  - bytes `[(y*W + x)*4 .. (y*W + x)*4 + 4)` — `img_in[y][x]`,
    row-major i32 LE.
- **`reference.bin`** (1024 bytes):
  - bytes `[(y*W + x)*4 .. (y*W + x)*4 + 4)` — `img_out[y][x]`,
    row-major i32 LE. Boundary cells (y in {0, H-1} or x in
    {0, W-1}) are zero.

Both fixtures are well under the 10 KB cap recommended by
[`docs/reference-impl-policy.md`](../../../docs/reference-impl-policy.md)
§1 — small enough to read in `xxd` by hand.

The pattern used in `input.bin`:

```
img_in[y][x] = (y * 16 + x) * 7
```

A linear gradient: small enough to never overflow the 9-term sum
(max 15*16+15 = 255 -> 1785; nine such pixels sum to ~16k, well
inside i32). Linear-by-construction means the 3x3 mean equals the
centre pixel for the truly-interior pixels (where all nine
neighbours exist) — a clean spot-check: `img_out[1][1] == img_in[1][1]
== 17 * 7 = 119 = 0x77` (verify in `xxd reference.bin`).

## How to regenerate the fixtures

Per [policy §1](../../../docs/reference-impl-policy.md#1-file-layout):

```sh
# Regenerate input.bin (only if the I/O format changes).
python3 -c "
import struct
H, W = 16, 16
buf = bytearray()
for y in range(H):
    for x in range(W):
        buf += struct.pack('<i', (y * 16 + x) * 7)
open('nuc-nucleus/examples/05-stencil/input.bin', 'wb').write(buf)
"

# Regenerate reference.bin from the standalone reference impl.
cargo run --release \
  --manifest-path nuc-nucleus/examples/05-stencil/reference/Cargo.toml -- \
  --in    nuc-nucleus/examples/05-stencil/input.bin \
  --out   nuc-nucleus/examples/05-stencil/reference.bin
```

The committed `reference.bin` was produced by running the reference
binary on the committed `input.bin`. Spot-check: pixel `(1, 1)` of
the output should be `0x00000077` (LE: bytes `77 00 00 00` at offset
`(1*16 + 1)*4 = 68`).

## Reference implementation

`reference/` is a standalone Rust crate with no dependency on
Nucleus, on any backend crate, or on `kernels.rs`. See policy §2
(the independence rule); deliberate duplication of small reference
code across examples is part of the audit story.

It reads `input.bin`, decodes the 16x16 flat row-major i32 image,
computes `img_out[y][x]` for each interior pixel as the truncating
integer division by 9 of the nine-pixel sum, writes the flat
row-major output. No threads, no third-party crates, no `HashMap`
— determinism rule (policy §5).

## Numeric type choice: `i32`

PRD §10.1 — bit-identical tier-1 differential testing demands
deterministic numerics. Integer arithmetic is deterministic by
language definition; floating-point sum order is not, and a 3x3
box-blur IS a nine-element reduction whose order schedules will
reorder.

`i32::wrapping_add` documents the overflow contract. The committed
input fits comfortably within i32; the choice is defensive for
pathological inputs.

Trade-off: truncating integer division (`sum / 9`) loses precision
relative to a true mean. A 256-grey image becomes an 0..255 image
again; pixel values near the mean are mildly biased toward zero
(`-1`-truncate). This is a deliberate accuracy cost for determinism.
See "Honest limitations" below.

## Boundary handling

The algorithm loop is `for y : 1..H-1 { for x : 1..W-1 { ... } }` —
only interior pixels are written. The codegen pre-initialises
`img_out` to zero (per `apply_block_transforms.rs`'s `render_array_init`
behaviour: `vec![0i32; H*W]`). Boundary pixels therefore stay zero
in the output.

Alternative boundary policies considered and rejected:

- **Clamp/replicate** (`img_in[max(0, y-1)][...]`) — would require
  the kernel to know shape, or extra `min`/`max` machinery in the
  algorithm. Not in the v2 algorithm sublanguage (PRD §6.2.3 keeps
  indexing as integer arithmetic only).
- **Mirror** — same problem, plus a conditional that doesn't fit the
  affine-index restriction.
- **Skip-with-zero** (chosen) — composes with the single-assignment
  default and keeps the algorithm trivial. Cost: the outermost ring
  of pixels is "wrong" (zero, not blurred). For a stencil example
  this is documentary, not load-bearing.

The reference impl uses the same convention; the differential test
passes bit-identically without special-casing the boundary.

## Why `Vec<i32>` and not `[i32; H*W]` in `kernels.rs`?

Same reasoning as examples 01 / 02 / 03: TASK-0103. `[i32; H*W]`
would require Nuc-side `const H`, `const W` to be Rust consts in the
same file, which the PRD §6.2.2 example sketch does not specify
yet. `Vec<i32>` carries length at runtime; we check it explicitly in
`save_image`. Trade-off: shape errors become runtime panics rather
than compile-time mismatches. Resolves when TASK-0103 picks a
convention.

The codegen flattens the 2D `img_in[y][x]` to `img_in[y * W + x]` at
compile time, so the runtime layout is single flat row-major Vec.
File format MUST match that layout — the input generator above
writes row-major by construction.

## Contract-check limitation

Same shape as examples 01 / 02 / 03:

- **PASS** for `blur3` — declared `(i32, ..., i32) -> i32`, nine
  scalar params, scalar return. Matches the Rust function exactly.
- **`TypeMismatch`** for `load_image`, `save_image` — their
  Nuc-side declarations are aggregate (`i32[H][W]`) and the current
  matcher emits "aggregate type matching is not yet implemented".
  Intended behaviour at TASK-0012's scope, not a bug in the example.

The matching aggregate-pinning test in
`nucleus/nucleus-compiler/tests/contract.rs` pins exactly this behaviour for
this example's `kernels.rs`.

## End-to-end status (HONEST)

- **`naive` × `pthreads-sync`** — passes bit-identically against
  `reference.bin`. Carries the correctness gate for this example.
  Wired into `nuc-nucleus/e2e-matrix.toml` and into
  `nucleus/nucleus-compiler/tests/e2e_example_05.rs`.

- **`blocked` × `pthreads-sync`** — `#[ignore]`'d, with a TODO
  reference to TASK-0142 (trailing remainder tiles) + TASK-0143
  (per-tile transfer hoisting). The schedule asks `block=4` on `y`;
  the algorithm walks `y in 1..H-1 = 1..15`, range 14, NOT divisible
  by 4. The block-transform pass (TASK-0030) rejects non-divisible
  ranges with `BlockTransformError::NotDivisible` rather than
  emitting a partial tile. Once TASK-0142 lifts that restriction,
  this cell flips from `ignore` to active. Until then the schedule
  is exercised by the parser/sched_lower pinning tests only.

- **`distributed` × `pthreads-sync`** — schedule parses and lowers
  (existing pinning tests in `sched_parser.rs`, `sched_lower.rs`),
  but the pthreads-sync backend rejects distributed placement
  (TASK-0117 + TASK-0126). No e2e cell.

## Required schedules

- `naive.sched.nuc` — single worker (`host`); the load-bearing
  correctness gate at M2.
- `blocked.sched.nuc` — single worker (`host`) with `loop y :
  block=4;`. Exercises the block-transform pass structurally;
  full e2e is gated on TASK-0142 + TASK-0143.
- `distributed.sched.nuc` — four-worker stretch schedule. Exercises
  the parser / lowering / link pipeline for a multi-worker stencil;
  e2e gated on TASK-0117 + halo synthesis follow-ups.

## Honest limitations

1. **Truncating integer division loses precision** vs a "true
   average". This is the cost of bit-deterministic output across
   schedules and backends. PRD §10.1 demands the latter; we pay the
   precision cost.

2. **Boundary is zero, not blurred.** The outermost ring is the
   single-assignment-default. Acceptable for an example; not what a
   real image processing pipeline would do. PRD §6.2.3 doesn't
   support the conditional-index machinery a clamp/mirror policy
   needs.

3. **`block=4` on a range that is not a multiple of 4** is rejected
   by the block-transform pass — the schedule is intentionally
   inconsistent with the algorithm's `1..H-1` bound to flush out
   TASK-0142. The naive schedule carries the correctness gate; the
   blocked cell is `#[ignore]`'d.

4. **`reuse` is parser-coverage only.** The `distributed.sched.nuc`
   declares `loop x : block=64, vectorize=8, reuse;` for the
   structural pinning tests, but the `reuse` semantics (sliding-
   window register-reuse across consecutive iterations of `x`) is
   not yet wired into the compiler. Filed as a follow-up.

5. **No `f32` variant**, even though the original 2013 thesis
   example was float. PRD §12 narrows to integer arithmetic for v2
   determinism. A future `5b-stencil-f32` example would have to
   either pin a deterministic float reduction order or compare with
   epsilon — both are out of scope here.
