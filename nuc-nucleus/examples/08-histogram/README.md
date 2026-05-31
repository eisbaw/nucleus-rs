# Example 08 — Histogram

Array-output reduction with data-dependent indexing. Input is `N=256`
i32 LE values, each in `[0, BINS-1]` with `BINS=16`. Output is a
BINS-wide i32 LE array where `histogram[v]` is the count of input
elements equal to `v`. This is PRD §9 row 8 ("reduction with shared
output array"), distinct from example 03 (reduction-to-scalar) by
reducing to an array.

## What this example stresses

| Axis        | What                                                                |
| ----------- | ------------------------------------------------------------------- |
| Algorithmic | Array-output accumulator pattern (`histogram[b]` read+written in the same statement, indexed by inner loop variable); data-dependent indexing pushed to the kernel via a masked-increment shape (`bin_inc(acc, value, bin)` returns `acc+1` iff `value==bin`). |
| Scheduling  | Naive is BIT-IDENTICAL on all 4 tier-1 backends at TASK-0044.04 cycle 186. Distributed is a STRETCH — see "Required vs stretch schedules". |
| Backends    | `pthreads-sync` / `mp-tcp-bufsync` / `pthreads-async` / `mp-tcp-event` all bit-identical against `reference.bin` for the naive schedule. |

The example uses the **rectangular masked-accumulator** shape already
proved on examples 03-reduction and 04-prefix-sum (`exclusive_add(acc,
x, b, c)` and `block_scan(acc, x, boff, i, j)`). The conditional
("if `value == bin`") lives in the Rust kernel body (PRD §6.2.2:
arbitrary Rust), NOT in the algorithm language (PRD §6.2.4: no
conditionals).

## What this example does NOT stress

- **Bin indexing with floating-point / non-trivial binning.** The
  input fixture is pre-clipped to `[0, BINS-1]`; the kernel does an
  integer equality check, no division/modulo/range bucketing. A
  real-world histogram on unconstrained input would either
  pre-process the value into a bin index in a separate kernel, or
  do the bucketing inside `bin_inc` directly — both are kernel-level
  extensions, NOT algorithm-level changes.
- **Distributed schedule with cross-worker partial-histogram
  combine.** AC#3 of TASK-0044.04 carries that stretch; the
  schedule lowers cleanly on every tier-1 backend but the host-
  side combine emits last-write-wins instead of element-wise sum.
  This is the new compiler-level gap filed as TASK-0343. See
  "Required vs stretch schedules" below.
- **Float reductions.** PRD §10.1 demands bit-identity; only
  integer counts here.

## Required vs stretch schedules

| Schedule                | Status at TASK-0044.04 cycle 186 | Why |
| ----------------------- | -------------------------------- | --- |
| `naive.sched.nuc`       | **Required**, e2e bit-identical against `reference.bin` on all 4 tier-1 backends. | Single worker; same compiler path as 03-reduction/naive + 04-prefix-sum/naive. |
| `distributed.sched.nuc` | **Required** (promoted in TASK-0343 cycle 189; was a `[[skip]]`'d stretch). | The overlapping-write accumulator combine landed at the backend-common layer; bit-identical on all 4 tier-1 backends. |
| `scatter.sched.nuc`     | **Required** (TASK-0376), e2e bit-identical against `reference.bin` on all 7 tier-1 backends. | Native data-dependent WRITE `histogram[input[i]]` (the scatter; see "Native scatter variant" below). Single worker; drives `prog.scatter.algo.nuc` + `kernels.scatter.rs`. |

## I/O format

Binary little-endian `i32` words. `N = 256`, `BINS = 16`. Mirror the
`const` declarations in `prog.algo.nuc`.

- **`input.bin`** (1024 bytes): `N` LE `i32` words. Each value MUST be
  in `[0, BINS-1]`; the reference impl validates strictly (silent
  acceptance of out-of-range values would mask fixture drift since
  the rectangular nest's kernel returns `acc` when no bin matches —
  out-of-range values would silently miss every bin).
- **`reference.bin`** (64 bytes): `BINS` LE `i32` words — the
  histogram.

Both fixtures are committed binaries, well under the 10 KB inspectability
cap (per [`docs/reference-impl-policy.md`](../../../docs/reference-impl-policy.md)).

The input pattern is non-uniform by construction (so a bug that
swaps bins surfaces as a counts mismatch, not just a permutation):

```python
import struct
N = 256
BINS = 16
buf = bytearray()
for i in range(N):
    if i < 100:
        v = i % 7              # bins 0..6 see ~14 hits each
    else:
        v = (i + 5) % BINS     # spreads across all 16 bins
    buf += struct.pack('<i', v)
open('input.bin', 'wb').write(buf)
```

The committed fixture's histogram is
`[25, 25, 24, 24, 24, 23, 23, 9, 9, 10, 10, 10, 10, 10, 10, 10]`
(sum = 256 = N).

Regenerate the input with the Python script above.

## How to regenerate `reference.bin`

Per [policy §1](../../../docs/reference-impl-policy.md#1-file-layout):

```sh
cargo run --release \
  --manifest-path nuc-nucleus/examples/08-histogram/reference/Cargo.toml -- \
  --in    nuc-nucleus/examples/08-histogram/input.bin \
  --out   nuc-nucleus/examples/08-histogram/reference.bin
```

## Algorithm structure

The algorithm uses a rectangular accumulator over `(i, b)`:

```
for i : 0..N {
    for b : 0..BINS {
        histogram[b] <-- bin_inc(histogram[b], input[i], b);
    }
}
```

`histogram` is pre-initialised to zero by the codegen (additive
identity). Each `(i, b)` cell calls `bin_inc(histogram[b], input[i],
b)` which returns `histogram[b] + 1` iff `input[i] == b`, else
`histogram[b]`. The accumulator builds the full histogram across all
N inputs and all BINS bins. This is the same single-assignment
accumulator shape as examples 03-reduction and 04-prefix-sum; the
single-assignment rule (PRD §6.2.1) is on the SYMBOL `histogram`,
not on individual indices.

### Native scatter variant (`prog.scatter.algo.nuc`)

The masked-accumulator form above is one way to compute the histogram;
the `scatter` schedule drives the **native data-dependent WRITE** form
directly:

```
for i : 0..N {
    histogram[input[i]] <-- inc(histogram[input[i]]);
}
```

This is the LHS / WRITE analog of 17-spmv's RHS / READ gather
`x[col_idx[i][k]]`. TASK-0341.03.01 lifted the lowering gate that once
forced data-dependent indexing into the kernel body: `lower_index_expr`
admits a data-dependent index in array-subscript position (on BOTH
sides here — `allow_gather` is set for the LHS via `lower_indices` and
for the RHS via `lower_data_ref`), so the address `histogram[input[i]]`
is expressible at the algorithm surface. The backend renders the LHS as
a scatter store
`histogram[(input[(i) as usize]) as usize] = kernels::inc(...);`
(TASK-0376). It drops the `for b` scan and the equality mask: a single
`for i` updates exactly the bin each input names, O(N) instead of
O(N*BINS).

The earlier README revision claimed "the algorithm language only allows
loop-variable indices on LHS" — that was true at cycle 186 but became
false once TASK-0341.03.01 landed; the gate was the lowering pass, not
the grammar (the index always parsed as a full expression). The one
genuine remaining limit is a **computed local bin** (`bin =
bucket(input[i]); histogram[bin]++`): the DSL has no local variables /
scalar-producing statements inside a loop (PRD §6.2.4), so value->bin
bucketing on UNCONSTRAINED input still lives in a kernel. The scatter
variant works here because the committed `input.bin` is already
pre-clipped to `[0, BINS)`, so `input[i]` IS a valid bin index.

`histogram` appears on both sides of the `<--` at the same
data-dependent index (same-symbol read-modify-write). Single-assignment
(PRD §6.2.1) is on the SYMBOL, not the index, so the repeated indexed
write is legal. The codegen classifies it as an ACCUMULATE fan-in (NOT a
cumulative cross-iteration array): the self-read index is structurally
IDENTICAL to the LHS index, so the cumulative-array discriminator (which
keys on a SHIFTED self-read index, as in jacobi/game-of-life) does not
fire. `histogram` is pre-initialised to zero (additive identity). The
scatter output is bit-identical to the masked form and to
`reference.bin`. Single-worker only; a distributed scatter is a broaden
follow-up to TASK-0376.

## Contract-check limitation

Same scalar-only contract-pass limitation as examples 01 / 02 / 03 /
04. Running `check_kernels_contract` against this example produces:

- **PASS** for `bin_inc` — declared `(i32, i32, i32) -> i32`,
  matches.
- **`TypeMismatch`** for `load_input` — declared `() ->
  i32[N]`, aggregate-typed; matched against `Vec<i32>`; scalar-only
  matcher emits the "aggregate type matching not yet implemented"
  diagnostic.
- **`TypeMismatch`** for `save_output` — declared `(i32[BINS]) ->
  ()`, aggregate-typed; same caveat.

When aggregate matching lands (TASK-0103 picks the convention,
TASK-0012 follow-ups implement matching), this example needs no
change; the matcher learns to accept `Vec<i32>` (or whatever) as
`i32[N]` / `i32[BINS]`.

## Cross-references

- `nuc-nucleus/examples/03-reduction/` — the closest template;
  reduction-to-scalar (one output slot) vs this example's
  reduction-to-array (BINS output slots).
- `nuc-nucleus/examples/04-prefix-sum/` — the masked-accumulator
  pattern (`exclusive_add` / `block_scan`).
- PRD §9 row 8 — the spec entry.
- TASK-0044.04 (this example's tracker task) — cycle 186 landed
  AC#1 + AC#2 across all 4 tier-1 backends; AC#3 distributed is
  filed as TASK-0343 (cross-worker array-accumulator combine gap).
- TASK-0343 — the new compiler-level gap surfaced by this
  example's distributed schedule.
