# Example 03 — Reduction (sum)

Two-phase integer sum reduction. Input is N=256 i32 LE words laid out
as `NUM_WORKERS x PARTITION_SIZE` (4 x 64). Output is a single i32 LE
scalar `result = sum_i a[i]`.

## What this example stresses

| Axis        | What                                                                |
| ----------- | ------------------------------------------------------------------- |
| Algorithmic | Accumulating pattern in the algorithm sublanguage; tree-combine at the algorithm level. |
| Scheduling  | Naive (host-only) lands at M1 (TASK-0022). Distributed is a stretch — see "Required vs stretch schedules". |
| Backends    | `pthreads-sync` bit-identical against `reference.bin` for the naive schedule. |

This is the smallest example with an **accumulating dataflow shape** —
`partials[w] <-- accumulate(partials[w], a[w][i])` reads and writes the
same data slot inside a loop. The single-assignment rule (PRD §6.2.1)
is on the data symbol, not on every iteration of the enclosing loops;
the codegen pre-initialises `partials` to zero (the additive identity)
so the inner fold is well-defined.

The example also exposes a **partition axis** at the algorithm level
by giving `a` a 2D shape `i32[NUM_WORKERS][PARTITION_SIZE]`. A
distributed schedule can `partition=workers` over the outer loop;
TASK-0022 lands the naive schedule and ships the distributed schedule
as a stretch (see below).

## What this example does NOT stress

- **Float reductions.** PRD §10.1 invariant: bit-identity requires
  deterministic arithmetic. Float sum order matters. Integer only.
- **Min / max reductions.** They are associative under their algebraic
  identities (INT_MIN / INT_MAX), but those identities are not
  materialised at the algorithm level by the pre-init pass (which
  defaults to zero). v2 has no `init=` clause. Sum's identity is zero
  — which the pre-init pass already provides — so sum is what fits
  cleanly today. Min/max would require either explicit init kernels
  or a language change.
- **Distributed transfer codegen for per-partition slices.** TASK-0122
  ships whole-symbol transfers only; per-tile transfers belong to
  TASK-0126. The distributed schedule parses, lowers, and links — but
  the e2e binary cannot be emitted yet.
- **Async / buffered transfers**, **pipelining**, **stencils / halos**.
  Those surfaces belong to later examples (9, 11, 5, 6).

## Required vs stretch schedules

| Schedule                  | Status at TASK-0022 | Why                                                  |
| ------------------------- | ------------------- | ---------------------------------------------------- |
| `naive.sched.nuc`         | **Required**, e2e bit-identical against `reference.bin`. | Single worker; M1 backends already support this shape. |
| `distributed.sched.nuc`   | **Stretch**, e2e gated by `#[ignore]` with TODO. | Distributed placement triggers backend `UnsupportedFeature` (TASK-0117, TASK-0126). |

## I/O format

Binary little-endian `i32` words. `N = 256`, `NUM_WORKERS = 4`,
`PARTITION_SIZE = N / NUM_WORKERS = 64`. These mirror the `const`
declarations in `prog.algo.nuc`.

- **`input.bin`** (1024 bytes): `N` LE `i32` words. Row-major over
  the algorithm's `i32[NUM_WORKERS][PARTITION_SIZE]` shape — partition
  `w` occupies bytes `[4 * w * PARTITION_SIZE .. 4 * (w+1) * PARTITION_SIZE)`.
- **`reference.bin`** (4 bytes): single LE `i32` word — the scalar
  `result`.

Both fixtures are committed binaries, well under the 10 KB inspectability
cap (per [`docs/reference-impl-policy.md`](../../../docs/reference-impl-policy.md)).

The input pattern is

```
a[i] = (i * 7) % 1000 - 500
```

Chosen because:

- Deterministic and reproducible in four lines of Python.
- Varies across `i` (not constant, not monotonic), so a bug that
  drops elements or swaps partitions shows up in `result`.
- Stays comfortably inside the i32 range; the running sum (-12520
  for the committed fixture) never approaches overflow.

Regenerate `input.bin`:

```python
import struct
N = 256
buf = bytearray()
for i in range(N):
    buf += struct.pack('<i', (i * 7) % 1000 - 500)
open('input.bin', 'wb').write(buf)
```

## How to regenerate `reference.bin`

Per [policy §1](../../../docs/reference-impl-policy.md#1-file-layout):

```sh
cargo run --release \
  --manifest-path nuc-nucleus/examples/03-reduction/reference/Cargo.toml -- \
  --in    nuc-nucleus/examples/03-reduction/input.bin \
  --out   nuc-nucleus/examples/03-reduction/reference.bin
```

The committed `reference.bin` decodes to `result = -12520`.

## Reference implementation

`reference/` is a standalone Rust crate with no dependency on Nucleus,
on any backend crate, or on `kernels.rs`. See policy §2 (the
independence rule).

It reads `input.bin`, decodes the i32 array, runs Phase 1 (sequential
per-partition `wrapping_add` folds into `partials[0..4]`), runs
Phase 2 (tree `wrapping_add(partials[0],partials[1])` then mirror
then top-level combine), and writes the 4-byte LE result to the
output path. No threads, no third-party crates, no `HashMap`.

## Numeric type choice: `i32`

PRD §10.1 invariant. Integer addition is bit-deterministic under any
reordering — sum is associative *and* commutative. `wrapping_add`
documents the overflow contract; the committed fixture stays inside
the i32 range but the choice is defensive.

`u32` would have worked equally. `i32` matches Rust's idiomatic
default integer width.

## Two-phase shape: design notes

The algorithm-level expression of "fold an array into a scalar"
requires care under Nuc's single-assignment-per-data-symbol rule:

- **Phase 1 (per-partition accumulate).** `partials[w]` is the LHS
  of exactly **one** dataflow statement (nested inside `for w`,
  `for i`). The codegen's "assignment counts per data symbol" walker
  records `partials` as `indexed`-only; the pre-init pass allocates
  `vec![0i32; NUM_WORKERS]` ahead of the loops. The fold then reads
  the previous value via `partials[(w) as usize]` on the RHS and
  writes the new value on the LHS — both indexed by the same
  iteration variable. Rust's borrow checker is happy because `i32`
  is `Copy` (the read returns a value, the write opens a fresh
  mutable borrow).

- **Phase 2 (tree combine).** Each intermediate scalar (`half1`,
  `half2`) is a fresh `data` declaration assigned exactly once. The
  final `result` is also assigned exactly once. The tree shape is
  written out explicitly with NUM_WORKERS=4 — depth-2, three
  `combine` calls — rather than expressed as a loop, because folding
  a scalar inside a loop would violate single-assignment (the LHS
  scalar has no index to vary).

For a much larger NUM_WORKERS the explicit fan-in would grow ugly;
the right way to scale is a loop-fold over an `i32[NUM_WORKERS]`
indexed accumulator (same shape as Phase 1, one dimension fewer).
The first example to need that doesn't exist yet (NUM_WORKERS=4 is
the load-bearing default for tier-1 testing).

## Contract-check limitation

Same scalar-only contract-pass limitation as examples 01 / 02. Running
`check_kernels_contract` against this example produces:

- **PASS** for `accumulate` — declared `(i32, i32) -> i32`, matches.
- **PASS** for `combine`    — declared `(i32, i32) -> i32`, matches.
- **PASS** for `save_output` — declared `(i32) -> ()`, scalar in /
  unit out, matches.
- **`TypeMismatch`** for `load_input` — declared `() -> i32[NUM_WORKERS][PARTITION_SIZE]`,
  aggregate-typed, scalar-only matcher emits the "aggregate type
  matching not yet implemented" diagnostic. Loud failure, not
  silent acceptance.

When aggregate matching lands (TASK-0103 picks the convention,
TASK-0012 follow-ups implement matching), this example needs no
change; the matcher learns to accept `Vec<i32>` (or whatever) as
`i32[N][M]`.

## Required schedules

- `naive.sched.nuc` — single worker (`host`), every kernel placed
  there. No loop transforms, no transfers. **Required** for tier-1
  conformance on this example; the e2e gate
  (`nucleus/nucleus-compiler/tests/e2e_example_03.rs`) verifies bit-identity against
  `reference.bin`.

## Stretch schedules

- `distributed.sched.nuc` — host plus four compute workers, outer
  `w` loop partitioned (`partition=workers`) across `{w0, w1, w2, w3}`,
  `a` and `partials` declared as `sync` cross-worker transfers.
  Compiles up through link; emit currently fails with
  `UnsupportedFeature` because distributed placement requires
  iteration-space partitioning (TASK-0117) and per-tile transfer
  codegen (TASK-0126). The e2e test exists as `#[ignore]` with a
  TODO comment.
