# Example 01 — Element-wise add

The smallest possible end-to-end example. Two input arrays `a`, `b` of
equal length; output `c[i] = a[i] + b[i]`.

## What this example stresses

| Axis        | What                                                              |
| ----------- | ----------------------------------------------------------------- |
| Algorithmic | Per-element scalar kernel, single loop, single dataflow chain.    |
| Scheduling  | Naive only: every kernel on `host`. No transfers, no blocking.    |
| Backends    | Smoke test — the bar for "the algorithm/schedule split parses, lowers, links". |

This is the M0 / M1 smoke test for the model. If this example does
not parse, lower, link, and produce reference-matching bytes on a
tier-1 backend, nothing else can.

## What this example does NOT stress

- Multi-worker placement, distributed iteration. (Example 2 splits
  the add across workers.)
- Cross-worker transfers, buffering, async IO. (Examples 9, 11.)
- Reduction patterns or ordering-sensitive accumulation. (Examples
  3, 4, 8.)
- Halo regions, stencil reuse. (Examples 5, 6, 11.)

If you find yourself wanting any of those, reach for the right
example — don't bloat this one.

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
a[i] = i * 3 + 7
b[i] = (i ^ 0x5A) * 2 - 11
```

These are deliberately non-trivial (varied across `i`, not constant,
not symmetric) so a bug that e.g. drops the `b` argument or swaps
indices is observable in `c`.

## How to regenerate `reference.bin`

Per [policy §1](../../../docs/reference-impl-policy.md#1-file-layout):

```sh
cargo run --release \
  --manifest-path nuc-nucleus/examples/01-elementwise-add/reference/Cargo.toml -- \
  --in    nuc-nucleus/examples/01-elementwise-add/input.bin \
  --out   nuc-nucleus/examples/01-elementwise-add/reference.bin
```

`input.bin` itself is regenerated only if the I/O format changes.
The generator is short enough to be inlined in this README for
auditability:

```python
import struct
N = 256
buf = bytearray()
for i in range(N):
    buf += struct.pack('<i', i * 3 + 7)
for i in range(N):
    buf += struct.pack('<i', (i ^ 0x5A) * 2 - 11)
open('input.bin', 'wb').write(buf)
```

(Not committed as a script: a four-line hex pattern is cheaper to
read in prose than a separate file. If the pattern grows or the
example gains many variants, promote it to `regen-input.py`.)

## Reference implementation

`reference/` is a standalone Rust crate with no dependency on
Nucleus, on any backend crate, or on `kernels.rs`. See policy §2
(the independence rule).

It reads `input.bin`, decodes two arrays, computes `c[i] =
i32::wrapping_add(a[i], b[i])` left-to-right per index, and writes
`c` LE-encoded to the output path. No threads, no third-party
crates, no `HashMap` — determinism rule (policy §5).

## Numeric type choice: `i32`

PRD §10.1 calls for bit-identical tier-1 differential testing. The
PRD also notes (§13 "Bit-identical output across backends") that
integer arithmetic is trivially deterministic, while floating-point
becomes non-trivial under reordering.

This example performs no reduction, so `f32` could have worked
without controversy. We pick `i32` anyway:

1. **Consistency with later integer-only examples** (sum, prefix
   sum, histogram, sort) where the determinism argument actually
   bites. Mixing types across examples would invite "but example 1
   used f32, why can't I" requests for examples where it really
   matters.
2. **Wrapping semantics are explicit**: `i32::wrapping_add` makes
   the overflow behaviour part of the contract. There is no
   "fast-math", no FMA, no platform-dependent rounding.
3. **Fixture inspectability**: `i32` words are easier to read in a
   hex dump than `f32` bit patterns.

`u32` would have worked equally well. `i32` matches Rust's idiomatic
default integer width and the sample math doesn't overflow.

## Why `Vec<i32>` and not `[i32; N]` in `kernels.rs`?

The PRD §6.2.2 example uses `Box<[[f32; W]; H]>` where `W`/`H` are
Nuc-side `const` declarations. That signature does NOT compile as
plain Rust because `W`/`H` are not Rust constants. The PRD bug is
tracked as **TASK-0103**.

This example sidesteps the issue: the I/O kernels' Rust signatures
use `Vec<i32>` (length-carrying owned buffer) instead of `[i32; N]`.
`N` is duplicated as a `const N: usize = 256;` inside `kernels.rs`
for the runtime length assertion — a deliberate single-source-of-
truth violation, called out in the file header and resolved when
TASK-0103 picks a convention.

The scalar `add` kernel does not have this problem: `(i32, i32) ->
i32` compiles standalone.

## Contract-check limitation

The contract pass [`check_kernels_contract`][contract] (TASK-0012)
is scalar-only at present. Running it against this example produces:

- **PASS** for `add` — declared `(i32, i32) -> i32`, actual matches.
- **`TypeMismatch`** for `load_input`, `load_input_b`, `save_output`
  — their Nuc-side declarations are aggregate (`i32[N]`) and the
  current matcher emits a loud "aggregate type matching is not yet
  implemented" diagnostic. This is intended behaviour at TASK-0012's
  scope, not a bug in the example.

When aggregate matching lands, the example does not need to change;
the matcher must learn to accept `Vec<i32>` (or whatever convention
TASK-0103 picks) as a valid Rust spelling of Nuc `i32[N]`.

[contract]: ../../../nucleus/compiler/src/contract.rs

## Required schedules

- `naive.sched.nuc` — single worker (`host`), every kernel placed
  there. No loop transforms, no transfers. This is the only schedule
  required for tier-1 conformance on this example; example 2 picks
  up where multi-worker decomposition starts.
