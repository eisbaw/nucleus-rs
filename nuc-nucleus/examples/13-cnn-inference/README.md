# Example 13 — Small CNN Inference

Layer-wise dataflow demonstrating the algorithm/schedule split across
three radically different decomposition patterns. One algorithm
(`prog.algo.nuc`), three schedules, same output everywhere.

## What this stresses

| Axis | What                                                         |
| ---- | ------------------------------------------------------------ |
| Algorithmic | Multi-stage layer dataflow. Per-sample loop nest.     |
| Scheduling  | Three orthogonal decompositions of the same algorithm: serial, batch-parallel, pipeline-parallel. |
| Backends    | All three schedules must work on every tier-1 backend, on MPI for tier 2, and in Renode on tier 3. |

This is the load-bearing demonstration of the v2 pitch: same algorithm,
different schedules, same correct output, across radically different
transports.

## Required schedules

All three are required for any backend to claim conformance on this
example.

- `naive.sched.nuc` — one worker, sequential batch. Smoke test.
- `batch_parallel.sched.nuc` — four compute workers; partition the
  batch loop.
- `pipeline_parallel.sched.nuc` — three compute workers; one layer
  per worker, three samples in flight.

## Why no training

Training requires backward-pass gradient synchronisation. The cheap
way is AllReduce; v2 emits only point-to-point in M7. Until collective
recognition lands (post-M8), training won't fit the model. Inference
is sufficient to demonstrate the schedule story, and is what most
deployed ML cares about anyway.

## Required schedules — current cycle status

Landed cycle-2 of TASK-0053 (post TASK-0209):

| Schedule          | Backend         | Status     | Notes                                            |
| ----------------- | --------------- | ---------- | ------------------------------------------------ |
| naive             | pthreads-sync   | REQUIRED   | byte-identical to `reference.bin`.               |
| naive             | mp-tcp-bufsync  | REQUIRED   | single-process under mp-tcp; same renderer.      |
| batch_parallel    | pthreads-sync   | SKIPPED    | TASK-0211 (multi-worker transfer distribution).  |
| batch_parallel    | mp-tcp-bufsync  | SKIPPED    | TASK-0175 + TASK-0211.                           |
| pipeline_parallel | (both)          | SKIPPED    | TASK-0210 (async+buffer+event tier-2).           |

`naive` exercises the load-bearing claim of this example: a single
algorithm with partial-rank dataflow into whole-layer kernels lowers
to a cargo-buildable backend crate (via TASK-0209's sub-array codegen)
and produces bit-identical output against a hand-written reference.
The other two schedules are tracked separately and re-enter the matrix
once the gaps close.

## Reference implementation

`reference/` is a standalone Rust crate with no dependency on
Nucleus, on any backend crate, or on `kernels.rs`. See policy §2
(the independence rule).

It reads `input.bin`, decodes a 4D tensor, applies the same algorithm
(`forward_conv_pool` for each conv block, then a per-class wrapping
dot product) and writes `reference.bin`. No threads, no third-party
crates, no `HashMap` — determinism rule (policy §5).

## Numeric type choice: `i32`

PRD §13 (open questions):

> Bit-identical output across backends. Trivial for integer algorithms;
> non-trivial once floating-point reductions enter (sum order matters).
> Either restrict examples to integer/deterministic FP, or compare with
> epsilon. Leaning toward integer-only for v2.

This example follows that lean: all activations, weights, conv sums,
and classifier logits are `i32`. The `f32` of a real CNN is NOT what
this example demonstrates — what it demonstrates is the layer-wise
dataflow shape (PRD §9 row 13) and the algorithm/schedule split.
Switching to `f32` would buy nothing for the shape demo while
introducing reduction-order non-determinism (FMA contraction, SIMD
reorder, fast-math platform variability).

`prog.algo.nuc` declares `data input : i32[B][C0][H][W]` etc.
`kernels.rs` and `reference/src/main.rs` independently compute the
same algorithm:

- **3x3 SAME-padded convolution** (zero-pad outside input bounds).
- **ReLU**: `max(0, x)`.
- **2x2 maxpool** stride 2.
- **Dense classifier**: per-class wrapping dot product over the
  flattened post-block-2 features.

Every reduction is a strict left-to-right `for` loop using
`i32::wrapping_mul` / `i32::wrapping_add`. Bit-deterministic by Rust's
language definition. No bias, no softmax — raw logits.

### Range / overflow

With input range `[-8, 7]`, conv layer 1 weights `[-2, 2]` and conv
layer 2 weights `[-2, 2]`:
- conv1 per-output sum: at most 1*9*16 = 144.
- conv2 per-output sum: at most 8*9*144*2 = 20736.
- classifier per-class sum: at most 784*20736*5 ≈ 81M.

All well inside `i32::MAX` (≈ 2.1B). `wrapping_*` documents the
overflow contract; pathological inputs do not panic.

## Weights

Both `kernels.rs` and `reference/src/main.rs` compute weights from
deterministic integer formulae keyed by `(oc, ic, ky, kx)` or
`(class, k)`. The formulae match bit-for-bit between the two files
but the implementations are independent (no shared crate; same
algorithm, separate source). A bug in one is not duplicated in the
other — the e2e differential against `reference.bin` is what catches
divergence.

The classifier modulus is `% 11` (not `% 5` like the conv layers).
Two properties motivated the choice (cycle-7 review-gate corrected):
**(1) symmetric output range** `[-5, 5]` — the 11 residues `0..10`
minus 5, centred on zero — important so the 784-tap dot product has
no systematic bias. `M = 10` would also produce distinct weight rows
(since `131 % 10 = 1` makes `(c * 131) % 10 = c` the identity on
`0..9`) but the range `[-5, 4]` is asymmetric by one. **(2) Primality
of 11** decouples weight-row uniqueness from the choice of multipliers
(131, 37) — for any non-zero multiplier pair, distinct `(class, k)`
pairs map to distinct residues. (Historical note: the prior comment
that `M = 11` was "the smallest M making `(class * 131) mod M`
injective on `0..9`" was mathematically false; M=10 is also
injective. The actual rationale is the symmetric-range argument
above.)

## I/O format

Binary little-endian `i32` words.

- **`input.bin`** (50176 bytes):
  - `B * C0 * H * W = 16 * 1 * 28 * 28 = 12544` LE i32 words.
  - Row-major flattening: byte offset of `input[n][c][y][x]` is
    `4 * (n*C0*H*W + c*H*W + y*W + x)`.
  - Generated by a deterministic per-index integer formula
    (see comments in `kernels.rs` and the generator script kept inline
    in this README below).

- **`reference.bin`** (640 bytes):
  - `B * N_CLASSES = 16 * 10 = 160` LE i32 words.
  - Per-sample classifier logits in row-major order.

### Regenerating `input.bin`

(only re-run if the I/O format changes — policy §4 "Forbidden
regeneration: the reference impl was re-run and produced a different
byte stream without any §3-class change.")

```python
import struct
B, C0, H, W = 16, 1, 28, 28
buf = bytearray()
for n in range(B):
    for c in range(C0):
        for y in range(H):
            for x in range(W):
                key = (n*65537 + c*257 + y*31 + x) ^ 0x5A
                v = (key % 16) - 8     # range [-8, 7]
                buf += struct.pack('<i', v)
open('input.bin', 'wb').write(buf)
```

### Regenerating `reference.bin`

```sh
cargo run --release \
  --manifest-path nuc-nucleus/examples/13-cnn-inference/reference/Cargo.toml -- \
  --in    nuc-nucleus/examples/13-cnn-inference/input.bin \
  --out   nuc-nucleus/examples/13-cnn-inference/reference.bin
```

## Honest limitations

- **No training.** Forward pass only. No backward pass, no gradients,
  no optimisers. PRD §9 row 13: "Demonstrates algorithm/schedule split,
  not training."
- **Tiny network.** B=16, two conv layers, one dense layer. Not a
  benchmark. The point is the dataflow shape, not throughput.
- **No quantisation.** `i32` activations are abstract integers, NOT
  fixed-Q quantised neural-net activations. A "real" quantised CNN
  would use Q15 or Q8 with explicit scale factors, saturating
  arithmetic, and per-channel calibration. None of that here.
- **No bias, no softmax.** Bias is implicit zero. Logits are raw
  `wrapping_add`/`wrapping_mul` sums. A real classifier post-softmax
  would feed cross-entropy loss; this one doesn't have a loss because
  there's no training.
- **Weight formula is hand-crafted, not learned.** Section §10.1's
  bit-identical differential is about the COMPILER faithfully
  realising an algorithm, NOT about the algorithm being a useful
  model. The weight formula is a deterministic generator chosen so
  that no two classes get identical weight rows and the accumulator
  ranges stay inside `i32`.
- **Only `naive` is differentially green this cycle.** The other two
  schedules track their own follow-up tasks (see Required schedules
  table above).

## What this example does *not* stress

- Halo regions (handled by example 5).
- Reuse on inner loops (example 5).
- Reduction patterns requiring collectives (example 3, partially).
- Wavefront / triangular dependencies (example 10).

If you find yourself wanting any of those, use the appropriate example
to test that axis — don't bloat this one.
