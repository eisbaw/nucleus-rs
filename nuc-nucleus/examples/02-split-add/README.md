# Example 02 — Element-wise add (split across two workers)

Same arithmetic as example 01 — `c[i] = a[i] + b[i]` over an array of
`N = 256` little-endian `i32` words — but the load-bearing schedule
splits the work across **two** workers:

- `host` runs the effectful I/O kernels (`load_input`,
  `load_input_b`, `save_output`). It owns the file system.
- `w0` runs the pure `add` kernel; the for-loop body executes there.

This is the smallest example in the matrix with a real cross-worker
data transfer.

## What this example stresses

| Axis        | What                                                                  |
| ----------- | --------------------------------------------------------------------- |
| Algorithmic | Identical to example 01: one scalar kernel + one for-loop + I/O.      |
| Scheduling  | The first **multi-worker** schedule and the first **`transfer`** directives. Arrays `a`, `b` cross `host -> w0`; array `c` crosses `w0 -> host`. |
| Backends    | (Once TASK-0122 lands) the first end-to-end exercise of Push/Wait codegen on `pthreads-sync`. |

In short: example 01 proves the algorithm/schedule split parses,
lowers, links, and runs end-to-end on a single worker. This example
proves the split survives a real cross-worker edge.

## What this example does NOT stress

- Distributed placement of `add` across many compute workers (`place
  add on { w0, w1, ... }`). A single `w0` runs the whole loop here.
- Blocking, vectorising, async / buffered transfers, halo regions,
  pipelining. Each of those gets its own example so the differential
  test isolates one axis at a time.
- Tier 2 (MPI) and tier 3 (embedded) backends. Tier 1 carries the
  formal claim (PRD §10.4); the others are engineering once tier 1
  is green.

## Files

```
02-split-add/
  prog.algo.nuc                 # algorithm (identical shape to example 01)
  kernels.rs                    # Rust bodies (i32 add, file-based I/O)
  schedules/
    naive.sched.nuc             # single-worker smoke test
    split.sched.nuc             # two-worker schedule — host + w0
  reference/                    # hand-written, std-only reference
    Cargo.toml
    src/main.rs
  input.bin                     # 2048 bytes — 256 i32 LE * 2 arrays
  reference.bin                 # 1024 bytes — expected c output
```

## I/O format

Binary little-endian `i32` words. `N = 256`; this matches `const N :
usize = 256;` in `prog.algo.nuc`.

- **`input.bin`** (2048 bytes):
  - bytes `[0      ..   4*N) ` — array `a`, `N` LE `i32` words.
  - bytes `[4*N    .. 4*2*N) ` — array `b`, `N` LE `i32` words.
- **`reference.bin`** (1024 bytes):
  - bytes `[0      ..   4*N) ` — array `c = a + b`, `N` LE `i32`.

The fixtures are committed binaries (per
[`docs/reference-impl-policy.md`](../../../docs/reference-impl-policy.md)
§1), each well under the 10 KB cap that keeps them inspectable by
hand (`hexdump -C input.bin | less`).

The pattern used in `input.bin`:

```
a[i] = (i * 5) - 13
b[i] = (i ^ 0xA5) + 41
```

Different generator from example 01 by design: each example's
fixtures are independent so a copy-paste error from example 01 into
this directory would be visible in the bytes (and in the
`reference.bin` SHA). The arithmetic still stays well within `i32`
range for `N = 256` — no overflow exercises here.

## How to regenerate `reference.bin`

Per [policy §1](../../../docs/reference-impl-policy.md#1-file-layout):

```sh
cargo run --release \
  --manifest-path nuc-nucleus/examples/02-split-add/reference/Cargo.toml -- \
  --in    nuc-nucleus/examples/02-split-add/input.bin \
  --out   nuc-nucleus/examples/02-split-add/reference.bin
```

`input.bin` itself is regenerated only if the I/O format changes.
The generator is short enough to be inlined here:

```python
import struct
N = 256
buf = bytearray()
for i in range(N):
    buf += struct.pack('<i', (i * 5) - 13)
for i in range(N):
    buf += struct.pack('<i', (i ^ 0xA5) + 41)
open('input.bin', 'wb').write(buf)
```

The committed `reference.bin` was produced by running the reference
binary on the committed `input.bin`. SHA-256 (informational, will
drift if the bytes drift):

```
dbc316d01531bbb4812cd317052fbce4c83b67ae02fe8ffdcba6622a42be9783
```

## Reference implementation

`reference/` is a standalone Rust crate with no dependency on
Nucleus, on any backend crate, on `kernels.rs`, or on example 01's
reference. See policy §2 (the independence rule); the deliberate
duplication of small reference code across examples is part of the
audit story.

It reads `input.bin`, decodes two arrays, computes `c[i] =
i32::wrapping_add(a[i], b[i])` left-to-right per index, and writes
`c` LE-encoded to the output path. No threads, no third-party
crates, no `HashMap` — determinism rule (policy §5).

## Schedule files

### `schedules/naive.sched.nuc` — smoke test

Single worker (`host`), every kernel placed there. No loop transforms,
no transfers. Same shape as `01-elementwise-add/schedules/naive.sched.nuc`.
Verifies that this example's `prog.algo.nuc` parses / lowers / links
under a one-worker schedule independent of the multi-worker split.

### `schedules/split.sched.nuc` — load-bearing

Two workers `{ host, w0 }`. `load_input`, `load_input_b`,
`save_output` placed on `host`; `add` placed on `w0`. Three
`transfer` directives mark the cross-worker data symbols:

```
transfer a : sync;   // host -> w0 (input)
transfer b : sync;   // host -> w0 (input)
transfer c : sync;   // w0 -> host (output)
```

All three are `sync` — minimum semantic the `pthreads-sync` backend
will provide once multi-worker codegen lands (TASK-0122). Async,
buffered, and notify-event transfers are exercised by later examples.

## Numeric type choice: `i32`

PRD §10.1 — bit-identical tier-1 differential testing wants
deterministic numerics; integer arithmetic is bit-deterministic by
language definition, floating-point reductions are not. This example
performs no reduction, so `f32` could have worked without controversy,
but `i32` matches the convention later integer-only examples (sum,
prefix sum, histogram, sort) actually need. Mixing types across
examples would invite "but example 1 / 2 used i32, why can't I" for
examples where determinism really bites.

`i32::wrapping_add` documents the overflow contract. There is no
"fast-math", no FMA, no platform-dependent rounding.

## Why `Vec<i32>` and not `[i32; N]` in `kernels.rs`?

Same reason as example 01: TASK-0103. `[i32; N]` would require the
Nuc-side `const N` to be a Rust const in the same file, which the PRD
§6.2.2 example sketch does not specify yet. `Vec<i32>` carries length
at runtime; we check it explicitly in `save_output`. Trade-off:
shape errors become runtime panics rather than compile-time
mismatches.

The scalar `add` kernel does not have this problem: `(i32, i32) ->
i32` compiles standalone.

## Contract-check limitation

Same shape as example 01:

- **PASS** for `add` — declared `(i32, i32) -> i32`, actual matches.
- **`TypeMismatch`** for `load_input`, `load_input_b`, `save_output`
  — their Nuc-side declarations are aggregate (`i32[N]`) and the
  current matcher emits "aggregate type matching is not yet
  implemented". Intended behaviour at TASK-0012's scope, not a bug
  in the example.

The matching aggregate-pinning test (`contract.rs`) pins exactly
this behaviour for this example's `kernels.rs`.

## End-to-end status (HONEST)

**Blocked on TASK-0122.** The current `pthreads-sync` backend rejects
multi-worker codegen with `EmitError::UnsupportedFeature("multi-worker
codegen not implemented at M1 ...")`. See TASK-0020's implementation
notes:

> Single-worker (naive) only at M1. Multi-worker codegen returns
> `EmitError::UnsupportedFeature(...)`. The synthetic two-worker
> ping-pong test that AC #5 of the original task description asks for
> is therefore *not* implemented as a positive test; it appears in
> `tests/emit.rs::multi_worker_is_rejected` as a *negative* test.

Concretely for this example:

- **Naive schedule** end-to-end (host-only, like example 01) WOULD
  run end-to-end through `pthreads-sync` today. We deliberately do
  NOT add such an e2e test — example 01 already covers that exact
  shape, and pinning it again here would be a redundant cell in the
  test matrix.
- **Split schedule** end-to-end is the value this example will
  deliver. It is currently blocked on TASK-0122 multi-worker codegen.
  The compiler test `e2e_example_02.rs::split_pthreads_sync_bit_identical`
  is `#[ignore]`'d and carries a `TODO` referencing TASK-0122 so the
  blocker has a single grep-able trail.

What this example DOES contribute today:

- The algorithm file `prog.algo.nuc` parses, lowers, and links under
  both schedules. The link pass is the load-bearing check for the
  multi-worker case — it verifies that the `transfer` directives
  satisfy the cross-worker dataflow requirement (omitting one is a
  compile error per PRD §6.3.4). Pinning tests in
  `nucleus/nucleus-compiler/tests/{algo_parser, algo_lower, sched_parser,
  sched_lower, link, contract}.rs` enforce all of this.
- The reference impl and committed `reference.bin` are ready for
  TASK-0122 to flip the e2e test from `#[ignore]` to active without
  needing any further example-side work.

## Required schedules

- `naive.sched.nuc` — single worker (`host`); smoke test.
- `split.sched.nuc` — two workers (`host`, `w0`); the load-bearing
  schedule for this example. Required for tier-1 conformance once
  multi-worker codegen lands.
