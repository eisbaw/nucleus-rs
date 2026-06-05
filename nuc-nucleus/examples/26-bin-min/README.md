# Example 26 — Bin min (distributed MIN-combine accumulator, non-zero identity)

Per-bin **minimum**. Two input streams of length `N=256` i32 LE:

- `key[i]` in `[0, BINS-1]` with `BINS=16` (input.bin words `[0..N)`).
- `val[i]` strictly **positive** i32 (input.bin words `[N..2N)`).

Output is a BINS-wide i32 LE array where

```
result[b] = min { val[i] : key[i] == b }      (or i32::MAX if bin b is EMPTY)
```

This is the **MIN sibling of 08-histogram** (which *sums* per-bin hits)
and the **non-zero-identity sibling of 25-bin-parity** (which XOR-folds,
identity 0). It exists to exercise the **identity-aware accumulator
pre-init** landed in TASK-0343.01.02: the kernel `bin_min` declares
`combine=min`, whose algebraic identity is **`i32::MAX`** — NOT zero. So
every accumulator pre-init site (the per-worker partial AND the host
accumulator) across all 7 tier-1 backends must emit `vec![i32::MAX; ...]`,
and the host element-wise combine emits the **method form**
`result[_k] = result[_k].min(_tmp[_k])`.

## What this example stresses

| Axis        | What                                                                |
| ----------- | ------------------------------------------------------------------- |
| Algorithmic | Array-output accumulator with a **non-zero** `combine=min` identity; data-dependent indexing pushed to the kernel via a masked min-fold (`bin_min(acc, k, v, bin)` returns `min(acc, v)` iff `k==bin`). |
| Scheduling  | `naive` (single-worker smoke — already exercises the identity init) + `distributed` (i-band partition over the OUTER input index, whole-array replicate of `result` pre-init `i32::MAX`, host element-wise MIN-combine — mirrors 08-histogram/distributed exactly, only the combine op + its identity differ). |
| Backends    | Both schedules bit-identical against `reference.bin` across **all 7 tier-1 backends**. |

## Why this fixture is maximally sensitive to a missed init site

ALL `val` are strictly positive and bin 15 is deliberately **EMPTY**.
So a backend that *wrongly* pre-inits the accumulator to `0` (the old
hardcoded zero) instead of the `min` identity `i32::MAX` diverges on
**every** output element:

- every **non-empty** bin's correct min is positive, but a 0-init yields
  `min(0, positive) == 0`;
- the **empty** bin must read `i32::MAX`, but a 0-init yields `0`.

A wrong/missing identity init on **any one** of the 7 backends therefore
breaks the 7-way bit-identity differential — that divergence *is* the
proof that no init site was missed. (During development this caught a
real defect: the `pthreads-async` multi-worker init was the
structurally-identical silent sibling that still emitted `vec![0; …]`;
it was routed through the shared identity helper in the same cycle.)

## Why the distributed MIN-combine is sound

The outer `i` loop is partitioned across `w0..w3` (`partition=workers`).
Each worker min-folds **only its i-band** into a **private full-width**
`result` (whole-array replicate, pre-init `i32::MAX`), then pushes the
full partial to the host. The host min-combines the four partials
element-wise.

`min` is **associative + commutative**, and the i-band partition is a
**disjoint cover** of the inputs. A worker whose band contains no
`key==b` keeps `result[b] == i32::MAX` (the identity), so the host's
`min(i32::MAX, …)` correctly ignores it. The element-wise min of the
four full partials equals the global per-bin minimum, **independent of
worker arrival order** ⇒ bit-identical across all 7 backends (PRD §10.1)
and equals the independent reference oracle.

## Out of scope

- **max / and**: same identity-aware machinery, different identity
  (`max`→`i32::MIN`, `and`→all-ones `!0T`); `min` is the
  maximally-sensitive demo and the one wired here.
- **Float reductions**: PRD §10.1 demands bit-identity; integer `min`
  only (total order, no NaN).

## Files

- `prog.algo.nuc` — the algorithm (rectangular masked min-fold nest).
- `kernels.rs` — `bin_min` (masked min-fold) + two-input I/O kernels.
- `schedules/naive.sched.nuc` — single-worker host smoke.
- `schedules/distributed.sched.nuc` — 4-worker i-band partition + host
  MIN-combine.
- `reference/` — independent oracle (`result[key[i]] = min(result[key[i]],
  val[i])` direct-index fold — structurally different from the masked
  rectangular nest).
- `input.bin` / `reference.bin` — fixture (256 keys over bins 0..14, bin
  15 empty; 256 positive vals) + expected output.

Regenerate `reference.bin`:

```
cargo run --release \
  --manifest-path nuc-nucleus/examples/26-bin-min/reference/Cargo.toml -- \
  --in  nuc-nucleus/examples/26-bin-min/input.bin \
  --out nuc-nucleus/examples/26-bin-min/reference.bin
```
