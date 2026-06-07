# 28-bin-fsum — reproducible distributed FLOAT SUM (TASK-0453.03)

Per-bin float sum: for each input element `i` with integer key
`key[i] ∈ [0, BINS)` and float value `val[i]`, add `val[i]` into
`result[key[i]]`. The float-sum sibling of `27-bin-fmin`.

## What it proves

A **distributed float sum** that carries the PRD §10.1 bit-identity
guarantee. Plain `combine=sum` on an `f32` accumulator is rejected
(IEEE-754 addition is non-associative, so an order-varying host fan-in
would diverge across backends). `bin_fsum` instead declares the opt-in
`combine=fsum`: the host combines the per-worker partials with a `+`
fold in a **fixed, worker-id-sorted order** (the deterministic host
event-list order every backend shares, TASK-0389), and each worker folds
its slice in ascending-index order. So all seven tier-1 backends produce
**byte-identical** output, matching the hand-written reference oracle,
which reproduces the identical partitioned fold (it moves in lockstep —
see `reference/src/main.rs`).

## The honest residual

`fsum` makes the chosen fold order *reproducible*, not the arithmetic
*exact*:

- It is cross-backend bit-identical **for this schedule**, not across
  schedules — a different worker count folds the partials differently.
- It is **not** the naive single-pass left-to-right IEEE sum; the
  partitioned association can round differently. The fixture's fractional
  values genuinely round, so the fold order is load-bearing (the oracle
  asserts the fixed fold differs from a naive single pass).
- It covers float **sum** only; `min`/`max` were already
  order-independent (`27-bin-fmin`).

## Distributed-only

This example ships a single `distributed` schedule. Unlike `27-bin-fmin`
(whose `min` combine is order-independent, so naive and distributed share
one `reference.bin`), the `fsum` fold is schedule-specific, so a naive
single-worker schedule would not match the distributed `reference.bin`.

## Regenerating the fixtures

`input.bin` packs a key stream (256 `i32` LE) then a val stream
(256 `f32` LE). Keys are `i % BINS`; vals are `1.0 + i*0.1` (strictly
positive, finite, and fractional so the sum genuinely rounds):

```python
import struct
N, BINS = 256, 16
buf = bytearray()
for i in range(N): buf += struct.pack('<i', i % BINS)
for i in range(N): buf += struct.pack('<f', 1.0 + i*0.1)
open('input.bin', 'wb').write(buf)
```

Then regenerate `reference.bin` (the lockstep oracle, which also asserts
the fold order is load-bearing on this input):

```
cargo run --release --manifest-path reference/Cargo.toml -- \
  --in input.bin --out reference.bin
```
