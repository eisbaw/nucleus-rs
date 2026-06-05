# Example 27 — Bin fmin (distributed FLOAT MIN-combine accumulator)

Per-bin **minimum over an f32 stream**. The float (`f32`) analog of
example 26-bin-min. Two input streams of length `N=256`:

- `key[i]` in `[0, BINS-1]` with `BINS=16`, i32 LE (input.bin words `[0..N)`).
- `val[i]` strictly **positive**, **finite**, **NaN-free** f32 LE
  (input.bin words `[N..2N)`).

Output is a BINS-wide **f32 LE** array where

```
result[b] = min { val[i] : key[i] == b }    (or f32::INFINITY if bin b is EMPTY)
```

This exercises the **float arm of the order-independence combine
policy** landed in TASK-0343.02: the kernel `bin_fmin` declares
`combine=min`, whose float algebraic identity is **`f32::INFINITY`** —
NOT `0.0`, and NOT `f32::MAX` (the largest *finite* value, which would
wrongly clamp out a genuine `+INFINITY` input). So every accumulator
pre-init site (the per-worker partial AND the host accumulator) across
all 7 tier-1 backends emits `vec![f32::INFINITY; BINS]`, and the host
element-wise combine emits the **method form**
`result[_k] = result[_k].min(_tmp[_k])` (Rust `f32::min`).

## What this example stresses

| Axis        | What                                                                |
| ----------- | ------------------------------------------------------------------- |
| Algorithmic | f32 array-output accumulator with a **non-zero, float** `combine=min` identity (`f32::INFINITY`); data-dependent indexing pushed to the kernel via a masked min-fold (`bin_fmin(acc, k, v, bin)` returns `acc.min(v)` iff `k==bin`). |
| Scheduling  | `naive` (single-worker smoke — already exercises the float identity init) + `distributed` (i-band partition over the OUTER input index, whole-array replicate of `result` pre-init `f32::INFINITY`, host element-wise MIN-combine — mirrors 26-bin-min exactly, only the scalar type + its identity differ). |
| Backends    | Both schedules bit-identical against `reference.bin` across **all 7 tier-1 backends**. |

## Why float MIN is admissible (and float SUM is not)

PRD §10.1 requires **byte-identical** output across backends that reduce
the per-worker partials in **different orders**. A combine op is
admissible only if it is **order-independent** (associative +
commutative) on its scalar type:

- **`min` / `max` on float** are order-independent for **distinct finite
  non-NaN** values, so the reduced bits are reduction-order-independent
  → bit-identical across all 7 backends. **ADMITTED.**
- **`sum` on float** is **non-associative** under IEEE-754 (rounding
  depends on the addition order), so different per-backend reduction
  orders give different bits → **REJECTED** at emit with a typed error
  citing PRD §10.1. There is therefore **no float-sum e2e cell** — the
  reject is covered by a negative unit test
  (`float_rejects_sum_with_non_associativity_message` in
  `nucleus/backend-common/tests/wait_assign_accumulate.rs`).
- **Bitwise `or`/`xor`/`and` on float** are undefined → **REJECTED.**

A deterministic float-sum (Kahan / worker-id-sorted fold) is a possible
future facility but is **out of scope** here (filed under TASK-0343.02).

## NaN / signed-zero caveat (out of scope; this fixture avoids it)

The admissibility of float `min`/`max` rests on order-independence for
**distinct finite non-NaN** values. Rust `f32::min` "ignores NaN" and
treats `-0.0` / `+0.0` as **equal**, so a bin mixing `±0.0`, or an
all-NaN bin, is **NOT guaranteed bit-stable** under reordering — which
bit pattern surfaces can depend on the reduction order. That is an
explicitly **out-of-scope, documented caveat**. **The committed fixture
is NaN-free with distinct strictly-positive finite `val`**, so the
order-independence guarantee holds and all 7 backends produce identical
bytes.

## Why this fixture is maximally sensitive to a missed init site

ALL `val` are strictly positive and **bin 15 is deliberately EMPTY**. So
a backend that *wrongly* pre-inits the accumulator to `0.0` (the old
hardcoded zero) instead of the `min` identity `f32::INFINITY` diverges
on **every** output element:

- every **non-empty** bin's correct min is positive, but a 0.0-init
  yields `min(0.0, positive) == 0.0`;
- the **empty** bin (15) must read `f32::INFINITY` (LE bytes
  `00 00 80 7F`, bit pattern `0x7F800000`), but a 0.0-init yields `0.0`.

A wrong/missing identity init on **any one** of the 7 backends breaks
the 7-way bit-identity differential — that divergence **is** the proof
that no init site was missed. (A MAX-combine over positive values would
NOT catch a 0.0-init on a non-empty bin, so **MIN** is used here.)

## Files

| File | Role |
| ---- | ---- |
| `prog.algo.nuc` | Algorithm: two input streams + the BINS-wide f32 `combine=min` accumulator. |
| `kernels.rs` | Rust kernel bodies (`bin_fmin` masked min-fold; f32 LE I/O). |
| `schedules/naive.sched.nuc` | Single-worker smoke (still exercises the float identity init). |
| `schedules/distributed.sched.nuc` | i-band partition + whole-array replicate + host float MIN-combine. |
| `reference/` | Independent f32 oracle (direct index-fold, init `f32::INFINITY`). |
| `input.bin` | 256 i32 keys (bin 15 empty) + 256 distinct positive finite f32 vals. |
| `reference.bin` | Expected BINS f32 LE output (bin 15 = `f32::INFINITY`). |

Regenerate `reference.bin`:

```
cargo run --release \
  --manifest-path nuc-nucleus/examples/27-bin-fmin/reference/Cargo.toml -- \
  --in  nuc-nucleus/examples/27-bin-fmin/input.bin \
  --out nuc-nucleus/examples/27-bin-fmin/reference.bin
```
