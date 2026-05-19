# Example 04 — Prefix sum (inclusive scan)

Inclusive prefix sum over `N = 256` i32 LE words, laid out as
`NB x BS` (`4 x 64`) so the block axis is exposed at the algorithm
level. Output: `out[k] = sum_{m=0..=k} in[m]`, `N` i32 LE words.

## What this example stresses

| Axis        | What                                                                 |
| ----------- | -------------------------------------------------------------------- |
| Algorithmic | A multi-pass dependency: the **ordering between three passes that share one worker** (block totals → exclusive block offsets → within-block scan + offset), each pass reading the previous pass's output. |
| Scheduling  | `naive` (host-only) is the required differential cell. `blocked` is shipped but skipped — see "Required vs skipped schedules". |
| Backends    | `naive` is **bit-identical against an independent `reference.bin`** under BOTH `pthreads-sync` and `mp-tcp-bufsync`. |

## The key design finding: scan is *not* directly expressible in v2

The textbook in-array carried recurrence
`out[i] <-- scan(out[i-1], in[i])` is **not expressible** in the
Nucleus v2 algorithm sublanguage. This was probed end-to-end (not
assumed) and is the headline finding of TASK-0039 (follow-up
**TASK-0179**):

1. **Boundary underflow, no guard.** `out[i-1]` lowers to
   `out[(i - 1) as usize]`; at `i = 0` that is `out[usize::MAX]` — an
   out-of-bounds panic. v2 has **no conditionals** (PRD §6.2.4) to
   special-case the first iteration.
2. **Single-assignment blocks the base-case split.** Writing it as
   `out[0] <-- copy(in[0]); for i:1..N { out[i] <-- scan(out[i-1],
   in[i]) }` is a **DoubleAssignment** — single-assignment is keyed on
   the data *symbol* (`out`), not the index (confirmed in
   `algo/lower.rs`).
3. **Loop bounds must be compile-time const.** The triangular
   reformulation `for j : 0..i+1` is rejected (the ACFG builder only
   `eval_const`s bounds, `acfg.rs:697`) — and it currently `panic!`s
   rather than returning a clean diagnostic (also tracked by
   TASK-0179).

The resolution keeps the algorithm to the **rectangular
reduction-accumulator** shape that example 03 already proves
bit-identical on both tier-1 backends, and pushes the
"which terms contribute / boundary" predicate into the hand-written
Rust **kernels** — which is the intended division of labour (PRD
§6.2.2: kernels say *what arithmetic*; the algorithm says *dataflow*).

## Algorithm: three passes, two ordering edges

```
Pass 1  block_sum[b]  = sum_{i} in[b][i]                    (reduction accumulator)
Pass 2  block_off[b]  = sum_{c < b} block_sum[c]            (reads Pass 1)
Pass 3  out[b][i]     = block_off[b] + sum_{j <= i} in[b][j] (reads Pass 2)
```

All three are rectangular accumulators with constant loop bounds,
one dataflow statement per data symbol (pre-init to 0 = additive
identity), and no carried index. The two read-after-write edges
(P1→P2, P2→P3) between passes sharing the single worker are exactly
the ordering this example is designed to stress.

The masking predicates live in the kernels:

- `accumulate(acc, x)` — Pass 1 fold.
- `exclusive_add(acc, x, b, c)` — adds `block_sum[c]` iff `c < b`.
- `block_scan(acc, x, boff, i, j)` — adds `in[b][j]` iff `j <= i`,
  and adds `boff` once (guarded by `j == 0`).

## What this example does NOT stress (honest limitations)

- **A parallel scan tree** (Blelloch / Hillis-Steele). v2 has no
  prefix-scan built-in (TASK-0039 AC#5); this is the sequential-style
  block decomposition. The within-block scan is O(BS²) and the
  block-offset prefix O(NB²) — fine for `N = 256`, not asymptotically
  optimal.
- **Distributing the blocks across workers.** The block axis is a
  clean partition handle, but per-tile transfer codegen is future
  work (cf. 03-reduction's distributed stretch). Shipped schedules
  are single-`host`.
- **Blocking.** A `blocked` schedule is shipped for documentation but
  is a **known-incorrect** cell (TASK-0180) — see below.
- **Float scans.** PRD §10.1: bit-identity needs deterministic
  arithmetic. Integer only; `wrapping_add` documents overflow intent.

## Required vs skipped schedules

| Schedule              | Status | Why |
| --------------------- | ------ | --- |
| `naive.sched.nuc`     | **Required**, e2e bit-identical vs `reference.bin` on `pthreads-sync` AND `mp-tcp-bufsync`. | Single worker; the three accumulator passes run sequentially. |
| `blocked.sched.nuc`   | **Skipped** (`[[skip]]` in `e2e-matrix.toml`; `#[ignore]`'d in `e2e_example_04.rs`). | `loop b : block=2` is *evenly divisible* (NB=4), but `b` is the loop variable of all three passes. The backend's `divisible_inner_block_vars` only rebinds an inner-block var whose loop occurs exactly once in the EventList (a guard for the non-divisible two-nest case, TASK-0173); a reused-name var trips count>1, rebinding is skipped, and the per-block accumulators **double-count** (output is 2x the reference on both backends). Tracked precisely as **TASK-0180**. It is skipped, NOT faked as required (TASK-0039 AC#3 honesty). |

The `blocked.sched.nuc` file is intentionally still shipped (it
parses, lowers, links, and builds) so TASK-0180 has a concrete
reproducer and so the schedule surface is documented.

## I/O format

Binary little-endian `i32` words. `N = 256`, `NB = 4`,
`BS = N / NB = 64` — these mirror the `const` declarations in
`prog.algo.nuc`.

- **`input.bin`** (1024 bytes): `N` LE `i32` words, row-major over
  the algorithm's `i32[NB][BS]` shape (block `b` occupies words
  `[b*BS .. (b+1)*BS)`).
- **`reference.bin`** (1024 bytes): `N` LE `i32` words — the inclusive
  prefix sums.

Both fixtures are committed binaries, well under the 10 KB
inspectability cap (see
[`docs/reference-impl-policy.md`](../../../docs/reference-impl-policy.md)).

The input pattern is `a[k] = (k * 7) % 1000 - 500` (the same family as
example 03): deterministic, varies across `k` (not constant, not
monotonic, so a dropped/swapped element shows up in the prefix sums),
and stays well inside the i32 range (values in `[-500, 499]`; the
running sum over `N = 256` never approaches overflow).

## How to regenerate the fixtures (no python — std-only Rust)

The nix dev shell has **no `python3`**; a python fixture step would be
non-reproducible and break `just`. The `reference/` crate is therefore
**both** the independent oracle **and** the fixture generator
(subcommand `--gen-input`). Regenerate in two steps:

```sh
# 1. input.bin — the canonical input pattern.
cargo run --release \
  --manifest-path nuc-nucleus/examples/04-prefix-sum/reference/Cargo.toml -- \
  --gen-input nuc-nucleus/examples/04-prefix-sum/input.bin

# 2. reference.bin — the independent prefix-sum oracle.
cargo run --release \
  --manifest-path nuc-nucleus/examples/04-prefix-sum/reference/Cargo.toml -- \
  --in  nuc-nucleus/examples/04-prefix-sum/input.bin \
  --out nuc-nucleus/examples/04-prefix-sum/reference.bin
```

The committed `reference.bin`'s first words decode to
`-500, -993, -1479, -1958, -2430, …`.

## Reference implementation

`reference/` is a standalone Rust crate with no dependency on
Nucleus, on any backend crate, or on `kernels.rs` (policy §2, the
independence rule). It computes the inclusive prefix sum a **second,
deliberately different way**: a single straight-line left-to-right
running total (`acc = acc.wrapping_add(in[k]); out[k] = acc`), NOT a
block decomposition. A backend whose block-decomposed output matches
this naive running sum bit-for-bit is unlikely to be "wrong in the
same way" as the reference. std only; no threads, no third-party
crates, no `HashMap`, no `Instant`.

## Numeric type choice: `i32`

PRD §10.1 invariant. Integer addition is bit-deterministic under any
reordering. `wrapping_add` documents the overflow contract; the
committed fixture stays in-range but the choice is defensive. `u32`
would have worked equally; `i32` matches Rust's idiomatic default.

## Contract-check limitation

Same scalar-only contract-pass limitation as examples 01/02/03/05.
The scalar step kernels (`accumulate`, `exclusive_add`, `block_scan`)
**PASS**; the aggregate-typed I/O kernels (`load_input` declared
`() -> i32[NB][BS]`, `save_output` declared `(i32[NB][BS]) -> ()`)
surface the known `TypeMismatch` (aggregate matching not yet
implemented — TASK-0012 / TASK-0103). Loud failure, not silent
acceptance; the build proceeds because it is a documented known gap.
