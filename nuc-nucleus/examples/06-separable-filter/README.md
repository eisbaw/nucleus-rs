# Example 06 — 5x5 separable box filter (two-pass)

A 5x5 box filter applied as a horizontal 1x5 pass followed by a
vertical 5x1 pass over a `16 x 16` i32 image. Pass 1 writes a
single-assignment **intermediate** `tmp`; Pass 2 consumes it. Output
is the 5x5 clamp-to-edge box **sum** (not average — no divide), `N =
256` i32 LE words.

## What this example stresses

| Axis        | What                                                                 |
| ----------- | -------------------------------------------------------------------- |
| Algorithmic | The **lifetime of an intermediate buffer across two passes that share a worker**: `tmp` has exactly one producer (Pass 1) and one consumer (Pass 2). |
| Scheduling  | `naive` AND `blocked` are both required differential cells. `blocked` tiles each pass's distinct outer-row axis with `block=4` (H=16, evenly divisible). |
| Backends    | Both schedules are **bit-identical against an independent `reference.bin`** under BOTH `pthreads-sync` and `mp-tcp-bufsync`. |

## The intermediate buffer (AC#2)

`tmp : i32[H][W]` is assigned by exactly one dataflow statement
(Pass 1's `tmp[hy][hx] <-- hblur_acc(...)`) and read by exactly one
(Pass 2's `vblur_acc(out[vy][vx], tmp[vm][vx], ...)`). That satisfies
PRD §6.2.1 single-assignment within scope, and is the
producer→consumer dependency the example targets.

## Why the taps go through a rectangular accumulator + Rust clamp

A textbook separable filter writes shifted taps like
`in[y][x-2]`. In Nuc v2 that underflows `usize` at the left edge
(`in[y][(x-2) as usize]` → out of bounds), and v2 has **no
conditionals** (PRD §6.2.4) to clamp the tap. This is the same class
of limitation TASK-0039 hit and filed as **TASK-0179**.

So, exactly as example 04-prefix-sum does, the algorithm uses only
the **rectangular reduction-accumulator** shape (proven bit-identical
on both backends by examples 03 / 04) and the **clamp-to-edge tap
selection lives in the Rust kernels**: `tmp[y][x]` accumulates over
*every* column `k`, and `hblur_acc` adds `in[y][k]` only when `k` is
one of the five clamped horizontal taps of `x`. All indices stay
in-range, so nothing ever goes out of bounds. This is the intended
division of labour (PRD §6.2.2: kernels say *what arithmetic*, the
algorithm says *dataflow*).

**Boundary policy: CLAMP-to-edge (replicate)** — a tap that falls
outside the image is replaced by the nearest edge sample, and the
edge sample is counted once per out-of-range tap. This is
deliberately different from 05-stencil (which *skips* the boundary
ring, leaving it zero); doing clamp here exercises a different policy.

## blocked is correct here — a positive control for TASK-0180

04-prefix-sum/blocked is *skipped* because its loop variable `b` is
reused across three passes, which trips the backend's
`divisible_inner_block_vars` count==1 guard and double-counts the
accumulators (TASK-0180). This example gives each pass its **own**
outer-row variable (`hy` for Pass 1, `vy` for Pass 2). Each tiled
inner loop therefore occurs **exactly once** in the EventList, the
count==1 guard is satisfied, absolute-index rebinding **is** applied
(`(0 + hy__tile*4 + hy)` etc.), and the blocked output is
**bit-identical** to the naive schedule and to `reference.bin` on
both backends. So 06/blocked is the **positive control** that
confirms TASK-0180's diagnosis (reused-name is the trigger, not
blocking-an-accumulator per se).

`block=4` is chosen evenly divisible into H=16 (4 full tiles, no
remainder) per the TASK-0173 discipline.

## What this example does NOT stress (honest limitations, AC#5)

- **Clamp boundaries only.** No mirror/wrap/zero boundary modes (the
  one policy is clamp-to-edge).
- **No reuse-with-shift.** A real separable filter slides a 5-wide
  window and recycles 4 of 5 taps between adjacent outputs. v2 has no
  `reuse` loop option wired end-to-end; the rectangular accumulator
  recomputes all W (resp. H) contributions per output — O(W)/O(H) per
  pixel, correct but not O(1)-optimal.
- **Box sum, not average.** No divide, to avoid baking a
  rounding/precision choice into the fixture; the reference sums the
  same way. Integer-typed (`i32`, `wrapping_add`) per PRD §10.1
  (floats are reorderable / non-deterministic).
- **Fast-memory placement of `tmp`** — see AC#4 below.

## AC#4 — should `tmp` live in fast memory?

Open design question, answered by **deferral**: whether the
intermediate buffer should be hinted into fast/scratch memory is a
**schedule** concern (`place_data`, PRD §6.3), NOT an algorithm one.
The algorithm says only *what* (`tmp` is a single-assignment
intermediate); *where it lives* belongs in a schedule directive. The
shipped schedules are single-`host` and do not exercise `place_data`;
a future memory-hierarchy schedule would add `place_data tmp on
<fast-mem>` without touching `prog.algo.nuc`. Recording this as the
deliberate division of responsibility rather than an omission.

## I/O format

Binary little-endian `i32` words. `H = W = 16` (mirrors the `const`
declarations in `prog.algo.nuc`).

- **`input.bin`** (1024 bytes): `H*W` LE `i32` words, row-major
  (`v[y*W + x]`).
- **`reference.bin`** (1024 bytes): `H*W` LE `i32` words — the
  separable 5x5 clamp box sum.

Both fixtures are committed binaries, well under the 10 KB
inspectability cap (see
[`docs/reference-impl-policy.md`](../../../docs/reference-impl-policy.md)).

The input pattern is `img[y][x] = (y*13 + x*7) % 251 - 125`:
deterministic, varies in BOTH axes (so a transposed/dropped row or
column shows up in the filtered output), and stays well inside i32
(values in `[-125, 125]`; the 25-tap box sum never approaches
overflow).

## How to regenerate the fixtures (no python — std-only Rust)

The nix dev shell has **no `python3`**; a python fixture step would
be non-reproducible and break `just`. The `reference/` crate is
**both** the independent oracle **and** the fixture generator
(`--gen-input`):

```sh
# 1. input.bin — the canonical input pattern.
cargo run --release \
  --manifest-path nuc-nucleus/examples/06-separable-filter/reference/Cargo.toml -- \
  --gen-input nuc-nucleus/examples/06-separable-filter/input.bin

# 2. reference.bin — the independent separable-filter oracle.
cargo run --release \
  --manifest-path nuc-nucleus/examples/06-separable-filter/reference/Cargo.toml -- \
  --in  nuc-nucleus/examples/06-separable-filter/input.bin \
  --out nuc-nucleus/examples/06-separable-filter/reference.bin
```

The committed `reference.bin`'s first words decode to
`-2825, -2720, -2580, -2405, …`.

## Reference implementation

`reference/` is a standalone Rust crate with no dependency on
Nucleus, on any backend crate, or on `kernels.rs` (policy §2). It
computes the separable box sum a **deliberately different way**: two
explicit passes that, for each output, sum exactly the five
clamp-to-edge taps via a small `for off in -2..=2` loop and an
explicit `clamp` — the textbook stencil, NOT the
visit-every-column/row-and-mask accumulator the Nucleus program uses.
A backend whose masked-accumulator output matches this explicit-tap
oracle bit-for-bit is unlikely to be "wrong in the same way". std
only; no threads, no third-party crates, no `HashMap`.

## Contract-check limitation

Same scalar-only contract-pass limitation as examples 01–05. The
scalar step kernels (`hblur_acc`, `vblur_acc`) **PASS**; the
aggregate-typed I/O kernels (`load_image` `() -> i32[H][W]`,
`save_image` `(i32[H][W]) -> ()`) surface the known `TypeMismatch`
(aggregate matching not yet implemented — TASK-0012 / TASK-0103).
Loud failure, not silent acceptance; the build proceeds because it is
a documented known gap.
