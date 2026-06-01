# Example 15 — Matrix transpose

Landed cycle 204 (TASK-0341.01 AC#1 language-sanity slice). Pure
axis-swap permutation over a 2D integer array.

`out[j][i] = in[i][j]` for `i in 0..H`, `j in 0..W`, with `H = 8`,
`W = 16`.

## What this example stresses

| Axis        | What                                                                           |
| ----------- | ------------------------------------------------------------------------------ |
| Algorithmic | 2D access on non-square dimensions with permuted LHS and RHS indices.          |
| Scheduling  | `naive` (single-worker `host`) and `distributed-rows` (4 compute workers).     |
| Backends    | All 7 tier-1 backends [[required]] on both schedules (naive + distributed-rows). |

This is the smallest possible fixture for "output-axis disagrees with
input-axis" — no other shipped example has this structural shape (01
is 1D unit-stride; 05 is 2D but each output row reads input rows
above/below in unit-stride, not transposed columns).

## What this example also demonstrates (post-AC#1)

- **Multi-worker placement / row-band partition of the output's row
  axis.** `distributed-rows.sched.nuc` uses `loop j : partition=workers`
  to row-band the output's row axis (the inner j-loop). This is the
  first shipped schedule to target an inner loop with
  `partition=workers` — `partition_workers::find_loop`'s recursive
  descent reaches it directly. The TASK-0301/TASK-0302 axis-mapping
  filter handles the resulting asymmetric per-data outcome (output
  j-banded gather, input whole-array broadcast) via the
  contiguous-prefix invariant biting on the axis-swap shape. Landed
  cycle 215 (TASK-0341.01.01) on pthreads-sync; cross-backend × 6
  siblings landed cycle 216 (TASK-0341.01.01.01); all 7 tier-1
  backends now [[required]] in the e2e matrix.
- **Cross-backend differential on the naive schedule.** Landed cycle
  205 (TASK-0341.01.02); all 7 tier-1 backends [[required]]
  bit-identical against `reference.bin`.

## What this example does NOT stress

- **Compute.** The kernel is an identity passthrough. The point of
  the example is the dataflow shape, not the arithmetic.

## I/O format

Binary little-endian `i32` words. `H = 8`, `W = 16`. Both arrays are
laid out row-major in their declared shape; numerically `H*W = W*H =
128` so input.bin and reference.bin are the same size.

- **`input.bin`** (512 bytes):
  - bytes `[(i*W + j)*4 .. (i*W + j)*4 + 4)` — `input[i][j]`,
    LE `i32`.
- **`reference.bin`** (512 bytes):
  - bytes `[(j*H + i)*4 .. (j*H + i)*4 + 4)` — `output[j][i]`,
    LE `i32`.

The fixtures are committed binaries (per
[`docs/reference-impl-policy.md`](../../../docs/reference-impl-policy.md)
§1), each well under the 10 KB cap that keeps them inspectable by hand
(`hexdump -C input.bin | less`).

The pattern used in `input.bin`:

```
in[i][j] = (i * 31 + j * 7) & 0xFF
```

These values vary across both `i` and `j` (so a bug that swaps the
loop variables, drops one axis, or transposes the wrong direction is
observable in the output). The `& 0xFF` keeps every word in `0..256`
so no wraparound is reachable from any cast.

## How to regenerate `reference.bin`

Per [policy §1](../../../docs/reference-impl-policy.md#1-file-layout):

```sh
cargo run --release \
  --manifest-path nuc-nucleus/examples/15-transpose/reference/Cargo.toml -- \
  --in    nuc-nucleus/examples/15-transpose/input.bin \
  --out   nuc-nucleus/examples/15-transpose/reference.bin
```

`input.bin` itself is regenerated only if the I/O format changes. The
generator is short enough to be inlined here for auditability:

```python
import struct
H, W = 8, 16
buf = bytearray()
for i in range(H):
    for j in range(W):
        val = (i * 31 + j * 7) & 0xFF
        buf += struct.pack('<i', val)
open('input.bin', 'wb').write(buf)
```

(Not committed as a script: the pattern is two lines of arithmetic —
cheaper to read in prose than a separate file.)

## Reference implementation

`reference/` is a standalone Rust crate with no dependency on
Nucleus, on any backend crate, or on `kernels.rs`. See policy §2.

It reads `input.bin`, decodes the `[H][W]` matrix, computes
`out[j][i] = in[i][j]` element-by-element, and writes `out` LE-encoded
in `[W][H]` row-major order. No threads, no third-party crates, no
HashMap.

## Numeric type choice: `i32`

PRD §10.1 calls for bit-identical tier-1 differential testing. The
transpose performs no arithmetic, so f32 would also be bit-deterministic
here. We pick `i32` anyway for consistency with examples 01/02/03/05.

## Why `Vec<i32>` and not `[i32; H*W]` / `[i32; W*H]`?

Per TASK-0103 (Done cycle 17), `Vec<i32>` + runtime length check is
the canonical convention for aggregate-typed kernel signatures. The
algorithm declares `input : i32[H][W]` and `output : i32[W][H]`. On
the Rust side both are flat `Vec<i32>`; the codegen flattens the 2D
index using the declared shape (`input[i][j]` -> `input[i * W + j]`,
`output[j][i]` -> `output[j * H + i]`).

The scalar `xpose : (i32) -> i32` does not have this problem.

## Why an identity kernel and not a bare `LValue` RHS?

The algorithm grammar
([`docs/grammar-algo.md`](../../../docs/grammar-algo.md) §1) allows a
bare `LValue` on the RHS of a dataflow assignment as an identity-copy
form:

```
RValue ::= CallExpr | LValue ;
```

But the ACFG builder [`acfg::build::build_dataflow`][acfg-reject]
REJECTS a non-`Call` RHS with a typed error
(`BuildAcfgError::KernelLessDataflowRhs`): a kernel-less identity-copy
`Operation` is not representable today (`Operation.kernel` /
`Event::Fire.kernel` are non-optional `KernelId`s, and no schedule
directive maps a data symbol to a worker set). So a bare `LValue` form
does not compile — an explicit kernel is required. A pure scalar kernel
returning its argument is the
canonical way to express "permute the indices and write the same
value"; the kernel form gives the compiler the per-element dataflow
node every shipped schedule shape reads from at cycle 204.

TASK-0111 (ACFG identity-copy handling) + TASK-0097 (the parallel
link-side gap) were closed cycle 77 as DEFERRED-until-real-example:
no shipped example used the bare-`LValue` identity-copy syntax at the
time. 15-transpose is that real example; the canonical co-design
follow-up (one task covering both layers, per the cycle-77 closure
note) is TASK-0347. TASK-0347 (cycle 230) landed the **link half**:
`link::dataflow::propagate_copy_edges` now attributes an identity
copy's producer/consumer transitively, so a cross-worker identity copy
is caught by the `MissingCrossWorkerTransfer` check. The **ACFG /
codegen half** — making `Operation` kernel-optional and resolving the
worker-set derivation blocker, after which this example could drop
`xpose` for the bare-`LValue` form — is filed as TASK-0360.

[acfg-reject]: ../../../nucleus/nucleus-compiler/src/acfg/build.rs

## Contract-check limitation

The contract pass [`check_kernels_contract`][contract] (TASK-0012) is
scalar-only at present:

- **PASS** for `xpose` — declared `(i32) -> i32`, scalar match.
- **`TypeMismatch`** for `load_input`, `save_output` — declared
  aggregate (`i32[H][W]` / `i32[W][H]`) and the current matcher emits
  a loud "aggregate type matching is not yet implemented" diagnostic.
  Intended behaviour at TASK-0012's scope, not a bug.

[contract]: ../../../nucleus/nucleus-compiler/src/contract.rs

## Required schedules

- `naive.sched.nuc` — single worker (`host`), every kernel placed
  there. No loop transforms, no transfers. The AC#1 conformance
  schedule.
- `distributed-rows.sched.nuc` — 4 compute workers
  (`{w0, w1, w2, w3}`) row-band partition the OUTPUT's row axis (the
  inner `j` loop). Each worker writes 4 contiguous output rows (W=16
  / 4 = 4, exact-divisible). Input is whole-array broadcast (the
  TASK-0301/TASK-0302 axis-mapping filter falls back to broadcast
  because `j` hits input's trailing dim after a hole at dim 0);
  output is `j`-banded on gather. Landed cycle 215 on pthreads-sync
  (TASK-0341.01.01); cross-backend promotion to all 7 tier-1 backends
  landed cycle 216 (TASK-0341.01.01.01); all 7 tier-1 backends
  [[required]].
