# Example 19 — histogram (truly-unconstrained input)

The dedicated fixture for the **TASK-0432 AC#2** residual: a textbook
histogram where the `bucket()` kernel does **real runtime work** because
the input is genuinely outside `[0, BINS)`.

## Why this example exists

`08-histogram/textbook` (TASK-0430) demonstrates a pure kernel call in
array-subscript **index** position:

```
histogram[bucket(input[i])] <-- inc(histogram[bucket(input[i])]);
```

But the `08` example shares ONE `input.bin` / `reference.bin` across all
its cells (the e2e harness uses one oracle per example dir), and that
`input.bin` is pre-clipped to `[0, BINS)`. So `bucket(v) == v` at runtime
there — the modulo is a **no-op**, and the unconstrained-input strength of
`bucket()` is shown only at the algorithm/codegen level, not at runtime.

This example ships its **own** `input.bin` with values genuinely outside
`[0, BINS)` (negatives and values `>= BINS`) and its **own**
`reference.bin` computed **through** the Euclidean-remainder bucket
`((v % BINS) + BINS) % BINS`. So `bucket()` does real work: a bare scatter
`histogram[input[i]]` would index out of bounds — only the bucketed index
lands in `[0, BINS)`. The output is bit-identical to the oracle **only if**
the compiled `bucket()`-in-index path actually evaluates the modulo.

The reference oracle (`reference/src/main.rs`) deliberately does **NOT**
validate `input ∈ [0, BINS)` (unlike `08-histogram`'s oracle, which rejects
out-of-range input) — folding through the modulo is the whole point. It
spells the bucket with `i32::rem_euclid` (structurally different from the
kernel's manual modulo, per `docs/reference-impl-policy.md` §2).

## Files

```
prog.textbook.algo.nuc                  # input `input`, output `histogram`; bucket()-in-index scatter
kernels.textbook.rs                     # bucket / inc / load_input / save_output (self-contained)
schedules/textbook.sched.nuc            # single-worker (host)
schedules/distributed.textbook.sched.nuc # host + w0..w3, input-index partition + element-wise-sum combine
input.bin                               # 1024 bytes — 256 i32 LE words, UNCONSTRAINED
reference.bin                           # 64 bytes — 16 i32 LE bins
reference/                              # independent std-only reference impl (rem_euclid bucket)
```

## Fixture (`input.bin`)

256 i32 LE words, generated deterministically with an `isqrt`-skewed bin
assignment offset into an unconstrained signed range. The result is an
intentionally **non-uniform** post-bucket distribution (a ramp
`1, 3, 5, …, 31` across bins 0..15, summing to 256) so the oracle genuinely
discriminates a correct bucketing from a wrong one (a uniform distribution
would not). It contains 111 negative values and 108 values `>= BINS`, with
only 37 already in `[0, BINS)` — so the bucket does real work on 219 of 256
elements.

```python
import struct, math
N, BINS = 256, 16

def gen(i):
    # bin assignment: isqrt(i) mod BINS -> a triangular skew across bins.
    b = math.isqrt(i) % BINS
    # pick a representative of residue class b that is OUT of [0, BINS):
    # add a multiple of 16 in {-3..3}*16 so values straddle 0 and BINS.
    return b + 16 * ((i % 7) - 3)

buf = bytearray()
for i in range(N):
    buf += struct.pack('<i', gen(i))
open('input.bin', 'wb').write(buf)
```

## Regenerate `reference.bin`

Via the project recipe (auto-discovers this example's reference crate):

```
just regen-references
```

or directly:

```
cargo run --release \
  --manifest-path nuc-nucleus/examples/19-histogram-unconstrained/reference/Cargo.toml -- \
  --in    nuc-nucleus/examples/19-histogram-unconstrained/input.bin \
  --out   nuc-nucleus/examples/19-histogram-unconstrained/reference.bin
```

`input.bin` and `reference.bin` agree by construction (the generator and
the oracle both compute the Euclidean-remainder bin). `reference.bin` is
byte-identical across the tier-1 backends for the registered schedules
(verified, TASK-0432).
