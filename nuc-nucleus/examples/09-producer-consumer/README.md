# Example 09 — Producer/consumer pipe

The smallest self-contained fixture for the M4 capability set: **async
transfer + buffer depth > 1 + notify=event + pipelined loop**. Two
pure kernels (`produce`, `transform`) in a one-dimensional dataflow
chain, scheduled either single-worker (smoke test) or as a buffered
pipeline across two compute workers (the M4 headline cell).

## What this example stresses

| Axis        | What                                                              |
| ----------- | ----------------------------------------------------------------- |
| Algorithmic | Two-stage producer/consumer chain; single cross-worker data symbol per iteration. |
| Scheduling  | `naive` (host-only smoke test on every tier-1 backend) and `pipelined` (`loop n : pipeline=4`, `transfer stream : async, buffer=4, notify=event`). |
| Backends    | `naive` × {pthreads-sync, mp-tcp-bufsync, pthreads-async} all bit-identical. `pipelined` × pthreads-async is the only cell whose capability surface matches; the other two are [[skip]] with the cited capability mismatch. |

This is the M4 cell PRD §9 row 9 calls out: "Producer/consumer pipe —
pipelining, buffer depth, async transfer". It is the smallest fixture
that exhibits buffered streaming between two compute workers.

## What this example does NOT stress

- **Halo regions, stencils, or cross-iteration data dependency.**
  Each `result[n]` depends only on `seeds[n]`. Stencil shape lives in
  examples 5, 6, 11.
- **Multi-stage pipelines (>2 compute stages).** Two stages is the
  smallest non-degenerate pipelined shape; 13-cnn-inference's
  `pipeline_parallel` schedule covers three (conv1 → conv2 →
  classifier).
- **Reductions, sorts, or any non-deterministic-under-reorder
  operation.** Examples 3, 4, 8 cover those.
- **Distributed placement of the producer or consumer.** A single
  producer and a single consumer is the smallest fixture for the
  pipelined capability surface; a multi-producer fan-in or
  multi-consumer fan-out is a separate example.

## I/O format

Binary little-endian `i32` words. `N = 16`; this matches `const N :
usize = 16;` in `prog.algo.nuc`.

- **`input.bin`** (64 bytes):
  - bytes `[0      ..   4*N) ` — array `seeds`, `N` LE `i32` words.
- **`reference.bin`** (64 bytes):
  - bytes `[0      ..   4*N) ` — array `result`, `N` LE `i32` words.

The committed input pattern is

```
seeds[i] = i + 1
```

i.e. the integers 1, 2, ..., 16. Non-zero (the seed=0 case would
zero the entire pipeline output and mask a class of "drop the
multiplier" bugs); strictly varying across `i` so a bug that drops
the iteration index or swaps two stream slots is observable in
`result.bin`.

Regenerate `input.bin`:

```python
import struct
N = 16
buf = bytearray()
for i in range(N):
    buf += struct.pack('<i', i + 1)
open('input.bin', 'wb').write(buf)
```

The committed `reference.bin` content is `result[n] = (n + 1) * 24`
(algebraically `transform(produce(s)) = (s*3)*7 + (s*3) = 24*s`),
i.e. `result = [24, 48, 72, 96, 120, ..., 384]`. The first word
hex-encodes as `18 00 00 00`; the last as `80 01 00 00` (0x180 = 384).

### Pinned hashes

```
input.bin     sha256: 77d735ce838418aa151bd96b5b1e78ee63860892e0a95c00fe34178442be9b07
reference.bin sha256: 354988c3e7d44caf339c24cbfa11ef1534e337751d99d7a4475202f2d6ccc86e
```

If `prog.algo.nuc`'s `const N`, `kernels.rs`'s `produce`/`transform`,
or the reference implementation's arithmetic ever changes, these
hashes change and the README must be updated in the SAME commit (per
policy §3).

### How to verify

The e2e harness diffs each backend's `output.bin` against
`reference.bin` (TASK-0023). To regenerate and re-verify by hand:

```sh
cargo run --release \
  --manifest-path nuc-nucleus/examples/09-producer-consumer/reference/Cargo.toml -- \
  --in    nuc-nucleus/examples/09-producer-consumer/input.bin \
  --out   nuc-nucleus/examples/09-producer-consumer/reference.bin

sha256sum nuc-nucleus/examples/09-producer-consumer/reference.bin
# expect: 354988c3e7d44caf339c24cbfa11ef1534e337751d99d7a4475202f2d6ccc86e
```

## Reference implementation

`reference/` is a standalone Rust crate with no dependency on
Nucleus, on any backend crate, or on `kernels.rs`. See
[policy §2](../../../docs/reference-impl-policy.md#2-independence)
(the independence rule).

It reads `input.bin`, decodes the seeds array, runs the same two
stages as the algorithm — `stream[n] = seeds[n].wrapping_mul(3)`,
`result[n] = stream[n].wrapping_mul(7).wrapping_add(stream[n])` —
left-to-right per index, and writes the result to the output path.
No threads, no third-party crates, no `HashMap` — determinism rule
(policy §5).

The reference deliberately keeps the TWO-STAGE shape rather than
folding to `result[n] = seeds[n] * 24`. The point of the reference is
to be a second, hand-audited witness with the SAME structural shape
as the algorithm and `kernels.rs`. If a bug drops or reorders one
stage in the Nucleus emit, the reference must NOT silently produce
the same wrong bytes — the third-witness argument requires
algorithmic similarity, not closed-form equivalence.

## Numeric type choice: `i32`

Same rationale as every other example. PRD §13 leans toward
integer-only for tier-1 differential testing; integer ops are
bit-deterministic by Rust's language definition. `wrapping_mul` /
`wrapping_add` document the overflow contract; the committed input
stays well inside the i32 range.

## Why `Vec<i32>` and not `[i32; N]` in `kernels.rs`?

Same as examples 01..07. TASK-0103 is the open PRD question for
aggregate-type matching. Until it lands, aggregate kernel I/O uses
`Vec<i32>` with a runtime length assertion in `save_output`.

## Contract-check limitation

The contract pass [`check_kernels_contract`](../../../nucleus/nucleus-compiler/src/contract.rs)
(TASK-0012) is scalar-only at present. Running it against this
example produces:

- **PASS** for `produce`   — declared `(i32) -> i32`, signature matches.
- **PASS** for `transform` — declared `(i32) -> i32`, signature matches.
- **`TypeMismatch`** for `load_input`, `save_output` — their Nuc-side
  declarations are aggregate (`i32[N]`) and the current matcher
  emits a loud "aggregate type matching is not yet implemented"
  diagnostic. Loud failure, not silent acceptance; same pattern as
  every other example.

When aggregate matching lands, this example does not need to change.

## Required schedules

| Schedule                  | Backends required at M3                     | Why                                                          |
| ------------------------- | ------------------------------------------- | ------------------------------------------------------------ |
| `naive.sched.nuc`         | `pthreads-sync`, `mp-tcp-bufsync`, `pthreads-async` | Single-worker smoke test; every tier-1 backend must produce bit-identical output. pthreads-async's single-worker arm delegates to the shared single-worker renderer. |
| `pipelined.sched.nuc`     | `pthreads-async` only (M4 cell)             | Requires `async + buffer=4 + notify=event`; only pthreads-async's capability surface satisfies this. The other two backends are [[skip]] in `e2e-matrix.toml` with the capability mismatch cited verbatim. |

The pipelined schedule places `produce` on `producer`, `transform` on
`consumer`, and runs the inter-stage edge as
`transfer stream : async, buffer=4, notify=event` with
`loop n : pipeline=4`. Four samples in flight at steady state.
