# Example 25 — Bin parity (distributed XOR-combine accumulator)

Per-bin **parity** (count mod 2). Input is `N=256` i32 LE values, each
in `[0, BINS-1]` with `BINS=16` (the SAME `input.bin` as 08-histogram).
Output is a BINS-wide i32 LE array where

```
parity[b] = (count of inputs equal to b) mod 2     (each 0 or 1)
```

This is the **XOR sibling of 08-histogram** (which *sums* per-bin hits).
It exists to exercise the **non-`wrapping_add` accumulator-combine arm**
landed in TASK-0343.01.01: the kernel `bin_xor` declares `combine=xor`,
so the cross-worker host combine emits the **operator form**
`parity[_k] = parity[_k] ^ _tmp[_k]` instead of the **method form**
`.wrapping_add(...)`.

## What this example stresses

| Axis        | What                                                                |
| ----------- | ------------------------------------------------------------------- |
| Algorithmic | Array-output accumulator with a `combine=xor` identity; data-dependent indexing pushed to the kernel via a masked-toggle (`bin_xor(acc, value, bin)` returns `acc ^ 1` iff `value==bin`). |
| Scheduling  | `naive` (single-worker smoke) + `distributed` (i-band partition over the OUTER input index, whole-array replicate of `parity`, host element-wise XOR-combine — mirrors 08-histogram/distributed exactly, only the combine op differs). |
| Backends    | `distributed` bit-identical against `reference.bin` across **all 7 tier-1 backends**. |

## Why the distributed XOR-combine is sound

The outer `i` loop is partitioned across `w0..w3` (`partition=workers`).
Each worker XOR-folds **only its i-band** into a **private full-width**
`parity` (whole-array replicate), then pushes the full partial to the
host. The host XORs the four partials element-wise.

XOR is **associative + commutative**, and the i-band partition is a
**disjoint cover** of the inputs — each input toggles its bin exactly
once, in exactly one worker's partial. So the element-wise XOR of the
four full partials equals the global per-bin parity, **independent of
worker arrival order** ⇒ bit-identical across all 7 backends (PRD §10.1).
This is a *stronger* determinism story than float sum (not associative-
stable). XOR shares the additive-identity **zero** with sum/or, so the
existing per-backend zero-init of `parity` is correct unchanged.

## Out of scope

- **Non-zero-identity combine** (`min`→MAX, `max`→MIN, `and`→all-ones):
  those need identity-aware init and are deferred to **TASK-0343.01.02**.
- **Float reductions**: PRD §10.1 demands bit-identity; integer XOR only.

## Files

- `prog.algo.nuc` — the algorithm (rectangular masked-toggle nest).
- `kernels.rs` — `bin_xor` (masked parity toggle) + I/O kernels.
- `schedules/naive.sched.nuc` — single-worker host smoke.
- `schedules/distributed.sched.nuc` — 4-worker i-band partition + host
  XOR-combine.
- `reference/` — independent oracle (`parity[v] ^= 1` direct-index
  toggle — structurally different from the masked rectangular nest).
- `input.bin` / `reference.bin` — fixture + expected output (parity =
  08-histogram counts mod 2; cross-checked).

Regenerate `reference.bin`:

```
cargo run --release \
  --manifest-path nuc-nucleus/examples/25-bin-parity/reference/Cargo.toml -- \
  --in  nuc-nucleus/examples/25-bin-parity/input.bin \
  --out nuc-nucleus/examples/25-bin-parity/reference.bin
```
