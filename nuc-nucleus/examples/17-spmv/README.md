# Example 17 — Sparse Matrix-Vector multiply (dense-stored)

Landed cycle 210 (TASK-0341.03 AC#1 language-sanity slice). Sparse
matrix-vector multiply on a dense-stored sparse matrix:

```
y[i] = sum_k val[i][k] * x[col_idx[i][k]]
```

for `i in 0..M`, `k in 0..NNZ`. NNZ=3 is fixed (variable nonzeros per
row is a data-dependent-loop-bound gap; see "What this example does
not stress" below).

The example is the companion to 08-histogram for the data-dependent-
indexing compiler-feature class. 08-histogram pins a data-dependent
WRITE address (`histogram[bin] <-- bin_inc(histogram[bin], value,
bin)` accumulates iff `value == bin`); 17-spmv pins a data-dependent
READ (`y[i]` accumulates `val[i][k] * x[col_idx[i][k]]`, where
`col_idx[i][k]` is a value loaded from a `data` array, not an
iv-affine expression). Both surface the same DSL grammar gap and
both rely on the same masked-accumulator workaround.

## What this example stresses

| Axis        | What                                                                  |
| ----------- | --------------------------------------------------------------------- |
| Algorithmic | Data-dependent indexing via rectangular masked-accumulator nest.      |
| Scheduling  | Naive only at AC#1: every kernel on `host`. No transfers, no blocks. |
| Backends    | pthreads-sync only at AC#1 (formal); informationally PASS elsewhere. |

The natural SpMV expression `y[i] = sum_k val[i][k] *
x[col_idx[i][k]]` is **grammatically inexpressible** in the v2
algorithm sublanguage. docs/grammar-algo.md fixes

```
IndexExpr ::= AddExpr
AddExpr   ::= MulExpr (('+'|'-') MulExpr)*
...
Atom      ::= IntLit | Ident | '(' AddExpr ')'
```

— an IndexExpr's Atom rule admits only IntLit / Ident /
parenthesised AddExpr; a nested IndexSuffix (`col_idx[i][k]`) inside
an IndexExpr (`x[...]`) is not in the grammar. AC#2 of TASK-0341.03
closes as honest-BLOCKED at the language-sanity boundary; the
missing primitive is filed as the AC#2 follow-up TASK-0341.03.01
(companion to TASK-0044.04's identical data-dependent-WRITE gap).

The workaround the algorithm uses instead is the rectangular masked-
accumulator nest:

```nuc
for i : 0 .. M {
    for k : 0 .. NNZ {
        for j : 0 .. N {
            y[i] <-- spmv_step(y[i], val[i][k], col_idx[i][k], x[j], j);
        }
    }
}
```

with `spmv_step(acc, v, c, x_j, j) = acc + v*x_j  iff j == c, else
acc`. Per (i, k) exactly one j matches `col_idx[i][k]`, so the
cumulative fold over `j in 0..N` equals `val[i][k] *
x[col_idx[i][k]]`. The kernel-side conditional is the same
data-dependent-comparison trick 08-histogram's `bin_inc` and
04-prefix-sum's `exclusive_add` already use.

## What this example does NOT stress (yet)

- **Variable nonzeros per row.** NNZ=3 is fixed; a true CSR with
  `row_ptr[i+1] - row_ptr[i]` per-row arity needs a data-dependent
  loop bound (the same gap as 16-jacobi's convergence variant,
  TASK-0341.02.01). Not in scope for this language-sanity slice.
- **Direct data-dependent read x[col_idx[i][k]] at the algorithm
  surface.** The DSL grammar gap; AC#2 honest-BLOCKED outcome with
  the precise missing primitive filed as TASK-0341.03.01.
- **Multi-worker distributed schedules.** Filed as a follow-up;
  same precedent as 15-transpose's AC#2 (TASK-0341.01.01) and
  16-jacobi's AC#3 (TASK-0341.02.02). partition=rows on the row
  index i is independent work per row (no cross-row dependencies),
  so the kernel-fan-out machinery already proven by
  08-histogram/distributed should suffice — but verifying against
  this shape is its own cycle.
- **Floating-point arithmetic.** Integer i32 with `wrapping_add` /
  `wrapping_mul` keeps the cross-backend differential
  bit-identical (PRD §10.1). Same trade as 05-stencil / 07-matmul /
  08-histogram / 16-jacobi.

## I/O format

Binary little-endian `i32` words. `M = 8`, `N = 8`, `NNZ = 3`.

- **`input.bin`** (224 bytes total):
  - bytes `[0   ..  96)` — `val[M][NNZ]`, row-major.
  - bytes `[96  .. 192)` — `col_idx[M][NNZ]`, row-major.
  - bytes `[192 .. 224)` — `x[N]`.
- **`reference.bin`** (32 bytes):
  - bytes `[0 .. 32)` — `y[M]`, the SpMV output.

Each fixture is well under the 10 KB cap that keeps fixtures
inspectable by hand (`hexdump -C input.bin | less`).

The pattern committed in `input.bin`:

```python
import struct
M, N, NNZ = 8, 8, 3
buf = bytearray()
# val[i][k] = (i+1) * (k+1)
for i in range(M):
    for k in range(NNZ):
        buf += struct.pack('<i', (i + 1) * (k + 1))
# col_idx[i][k] = (i+k) mod N
for i in range(M):
    for k in range(NNZ):
        buf += struct.pack('<i', (i + k) % N)
# x[j] = j+1
for j in range(N):
    buf += struct.pack('<i', j + 1)
open('input.bin', 'wb').write(buf)
```

Values are small positive integers; the worst-case row sum is
8 * 3 * 8 = 192, well under i32 range. Each row's col_idx walks
distinct columns so the masked accumulator hits a different j per k.
The pattern varies in both i and k so a transposed-loop bug is
observable.

Expected output (hand-checked, cycle 210):

```
y = [14, 40, 78, 128, 190, 264, 182, 128]
```

For row 0: `val=[1,2,3]`, `col_idx=[0,1,2]`, `x=[1,2,3,4,5,6,7,8]` →
`1*1 + 2*2 + 3*3 = 14`.

## How to regenerate `reference.bin`

Per [policy §1](../../../docs/reference-impl-policy.md#1-file-layout):

```sh
cargo run --release \
  --manifest-path nuc-nucleus/examples/17-spmv/reference/Cargo.toml -- \
  --in    nuc-nucleus/examples/17-spmv/input.bin \
  --out   nuc-nucleus/examples/17-spmv/reference.bin
```

`input.bin` is regenerated only if the I/O format changes (M, N, or
NNZ change). The generator is inline above for auditability.

## Reference implementation

`reference/` is a standalone Rust crate with no dependency on
Nucleus, on any backend crate, or on `kernels.rs`. See policy §2.

It reads `input.bin`, decodes `val[M][NNZ]`, `col_idx[M][NNZ]`,
`x[N]`, then computes `y[i] = sum_k val[i][k] * x[col_idx[i][k]]`
using direct array indexing (the natural SpMV form the algorithm
sublanguage cannot express). The reference rejects out-of-range
`col_idx` loudly; the algorithm's masked-accumulator would silently
sum zero on every j if no j matches an out-of-range index — that
divergence would mask a fixture drift, so we surface it here.

The reference uses `wrapping_mul` for the product and `wrapping_add`
for the accumulator to mirror `kernels.rs::spmv_step` exactly.

## Single-assignment shape (PRD §6.2.1)

Four top-level `data` symbols, each the LHS of exactly one Dataflow
statement at the statement level:

- `val`     — assigned once by `val <-- load_val();`.
- `col_idx` — assigned once by `col_idx <-- load_col_idx();`.
- `x`       — assigned once by `x <-- load_x();`.
- `y`       — assigned ONCE at the for-nest body level
  (`y[i] <-- spmv_step(...);`). The `(i, k, j)` iteration domain
  covers `i in 0..M`, `k in 0..NNZ`, `j in 0..N`; each `y[i]` is
  written `NNZ*N` times across the inner (k, j) walk. PRD §6.2.1
  single-assignment is on the data SYMBOL (one Dataflow statement
  assigns `y`), not on every iteration of the enclosing loops. The
  codegen pre-initialises `y` to zero (the additive identity for
  sum-of-products) so the (k, j) fold is well-defined. Same shape
  as 07-matmul's `c[i][j] <-- madd(c[i][j], a[i][k], b[k][j])`.

## Contract-check limitation

The contract pass `check_kernels_contract` (TASK-0012) is scalar-only
at present:

- **PASS** for `spmv_step` — five scalar i32 params, scalar i32 return.
- **`TypeMismatch`** for `load_val`, `load_col_idx`, `load_x`,
  `save_y` — declared aggregate (`i32[M][NNZ]`, `i32[N]`, `i32[M]`)
  and the current matcher emits a loud "aggregate type matching is
  not yet implemented" diagnostic. Intended behaviour at TASK-0012's
  scope, not a bug.

## Required schedules

- `naive.sched.nuc` — single worker (`host`), every kernel placed
  there. No loop transforms, no transfers. The only schedule required
  for AC#1 conformance.
