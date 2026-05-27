# Example 07 — Blocked integer matrix multiply

`C = A * B` over 16x16 `i32` matrices. For each `(i, j)`, output the
sum (with wrapping arithmetic) of `A[i][k] * B[k][j]` for `k` in
`0..N`. This is the PRD §9 row 7 example — the canonical 2D-blocking
and all-to-all communication stress.

## What this example stresses

| Axis        | What                                                                  |
| ----------- | --------------------------------------------------------------------- |
| Algorithmic | Triple-nested loop with a reduction on the innermost axis. The LHS `c[i][j]` appears on the RHS of the SAME dataflow statement — same accumulator pattern as example 03 (reduction), here in a deeper nest. Single-assignment (PRD §6.2.1) is on the data symbol; the codegen pre-init pass allocates `c` to zero, so the k-fold starts from the additive identity. |
| Scheduling  | 2D blocking: schedules can stack `block=` on both `i` and `j` (PRD §6.3.3). `schedules/blocked.sched.nuc` does exactly this — `loop i : block=8; loop j : block=8;` — turning the i/j grid into a 2x2 tile grid (16/8 = 2 per axis). The block-transform pass (TASK-0030) rewrites each loop independently. |
| Backends    | Naive cell: bit-identical against `reference.bin` under `pthreads-sync`. Blocked cell: `#[ignore]`'d on the e2e gate pending TASK-0143 (per-tile transfer hoisting). No remainder-tile dependency this time (N=16, block=8 divides cleanly). |
| Future      | All-to-all communication when distributed: any tile of C needs at least one full row of A and one full column of B. Not exercised in M2 (single worker) but the algorithm shape is the load-bearing setup for a future `partition=blocks2d` distributed schedule. |

## What this example does NOT stress

- Distributed placement of `madd` across compute workers
  (`place madd on { w0, w1, w2, w3 }` with all-to-all transfer
  synthesis). No `distributed.sched.nuc` shipped at this milestone —
  the supporting work (TASK-0117 + multi-tile transfer synthesis) is
  not in M2 scope. When that lands a four-worker schedule slots in
  here cleanly.
- `reuse` (block-row reuse of A or block-column reuse of B). The
  `block=` pass is structural only; reuse on the tiles is a future
  loop option (TASK-0144's scope).
- Floating-point arithmetic. PRD §10.1: integer for bit-identity.

## Files

```
07-matmul/
  prog.algo.nuc                 # v2 algorithm (signature-only kernels)
  kernels.rs                    # Rust bodies (i32 madd, file-based I/O)
  schedules/
    naive.sched.nuc             # single-worker smoke test (gate-passing)
    blocked.sched.nuc           # loop i : block=8; loop j : block=8 (e2e #[ignore]'d, see below)
  reference/                    # hand-written, std-only reference impl
    Cargo.toml
    src/main.rs
  input.bin                     # 2048 bytes — A then B, each 16*16 i32 LE
  reference.bin                 # 1024 bytes — expected C
```

## I/O format

Binary little-endian `i32` words. `N = 16`; matches `const N : usize
= 16;` in `prog.algo.nuc`.

- **`input.bin`** (2048 bytes):
  - bytes `[(i*N + j)*4 .. (i*N + j)*4 + 4)` — `A[i][j]`, row-major i32 LE.
  - bytes `[1024 + (i*N + j)*4 .. 1024 + (i*N + j)*4 + 4)` —
    `B[i][j]`, row-major i32 LE.
- **`reference.bin`** (1024 bytes):
  - bytes `[(i*N + j)*4 .. (i*N + j)*4 + 4)` — `C[i][j]`,
    row-major i32 LE.

The committed `input.bin` uses a bounded pattern that keeps all
accumulators well inside i32:

```
A[i][j] = ((i * 16 + j) % 13) - 6          # values in -6..=6
B[i][j] = (((i * 16 + j) * 7) % 13) - 6    # values in -6..=6
```

Per-element bound: `|x * y| <= 36`. Per-cell bound:
`|sum_k x*y| <= N * 36 = 576`. Observed max-abs in the committed
`reference.bin` is 112, comfortably inside i32. The pattern is
deterministic by construction so input.bin can be regenerated from
the formula (see below).

Spot-check (from `reference.bin`): `c[0][0] == 77` (LE bytes
`4d 00 00 00` at offset 0).

Both fixtures are well under the 10 KB cap recommended by
[`docs/reference-impl-policy.md`](../../../docs/reference-impl-policy.md)
§1 — small enough to read in `xxd` by hand.

## How to regenerate the fixtures

Per [policy §1](../../../docs/reference-impl-policy.md#1-file-layout):

```sh
# Regenerate input.bin (only if the I/O format changes).
python3 -c "
import struct
N = 16
buf = bytearray()
for i in range(N):
    for j in range(N):
        buf += struct.pack('<i', ((i*N+j) % 13) - 6)
for i in range(N):
    for j in range(N):
        buf += struct.pack('<i', (((i*N+j)*7) % 13) - 6)
open('nuc-nucleus/examples/07-matmul/input.bin', 'wb').write(buf)
"

# Regenerate reference.bin from the standalone reference impl.
cargo run --release \
  --manifest-path nuc-nucleus/examples/07-matmul/reference/Cargo.toml -- \
  --in    nuc-nucleus/examples/07-matmul/input.bin \
  --out   nuc-nucleus/examples/07-matmul/reference.bin
```

The committed `reference.bin` was produced by running the reference
binary on the committed `input.bin`.

## Reference implementation

`reference/` is a standalone Rust crate with no dependency on
Nucleus, on any backend crate, or on `kernels.rs`. See policy §2
(the independence rule); deliberate duplication of small reference
code across examples is part of the audit story.

It reads `input.bin`, decodes A and B from the two halves, runs the
plain `for i for j for k { c[i][j] += a[i][k] * b[k][j] }` triple
nest with `wrapping_mul` + `wrapping_add` (matching `kernels.rs::madd`
exactly), writes the flat row-major output. No threads, no
third-party crates, no `HashMap` — determinism rule (policy §5).

## Accumulator semantics — single-assignment + reduction loop

The dataflow line

```
c[i][j] <-- madd(c[i][j], a[i][k], b[k][j]);
```

inside the innermost `for k` loop writes `c[i][j]` N times across
the k-iterations. This is the SAME accumulator pattern example 03
(reduction) uses for `partials[w] <-- accumulate(partials[w],
a[w][i])`. PRD §6.2.1 single-assignment is on the data SYMBOL (one
dataflow statement assigns `c`), not on every iteration of the
enclosing loops. The codegen pre-init pass (in
`backends/pthreads-sync/src/lib.rs::collect_pre_init_data`)
allocates `c` as `vec![0i32; N*N]` before the loops run — this is
the zero from which the k-accumulator starts.

This is approach **(a)** from the task brief. We picked it over the
alternative **(b)** (explicit 3D temporary `partial[i][j][k]` with a
separate combine pass) because:

1. The compiler already accepts the pattern end-to-end (example 03
   proves this).
2. (b) would either need new language surface (a reduce-kernel
   primitive) or stage one more N^3 = 4096-element allocation per
   matmul.
3. (a) reads as plain code to a numerical programmer; (b) reads as
   compiler-pleaser code.

## Numeric type choice: `i32`

PRD §10.1 — bit-identical tier-1 differential testing demands
deterministic numerics. Integer arithmetic is deterministic by
language definition; floating-point sum-of-products order is not,
and matmul IS a sum-of-products reduction whose order schedules
will reorder (especially under 2D blocking).

`i32::wrapping_mul` + `i32::wrapping_add` document the overflow
contract. The committed input pattern fits comfortably; the choice
is defensive for pathological inputs.

Trade-off: integer matmul does not match a float matmul numerically.
For a real numerics workload a future variant would have to either
pin a deterministic float-reduction order or compare with epsilon —
both are out of scope here.

## Why `Vec<i32>` and not `[i32; N*N]` in `kernels.rs`?

Per TASK-0103 (Done cycle 17): `Vec<i32>` + runtime length check IS
the canonical convention for aggregate-typed kernel signatures. The
PRD §6.2.2 sketch `Box<[[f32; W]; H]>` did not compile as plain Rust
(W and H are not Rust constants); `Vec<i32>` with explicit length
checks in `save_c` is the resolution. Trade-off: shape errors become
runtime panics rather than compile-time mismatches.

The codegen flattens the 2D `c[i][j]` to `c[i * N + j]` at compile
time, so the runtime layout is single flat row-major Vec. File
format MUST match that layout — the input generator above writes
row-major by construction.

## Contract-check limitation

Same shape as examples 01 / 02 / 03 / 05:

- **PASS** for `madd` — declared `(i32, i32, i32) -> i32`, three
  scalar params, scalar return. Matches the Rust function exactly.
- **`TypeMismatch`** for `load_a`, `load_b`, `save_c` — their
  Nuc-side declarations are aggregate (`i32[N][N]`) and the current
  matcher emits "aggregate type matching is not yet implemented".
  Intended behaviour at TASK-0012's scope, not a bug in the example.

The matching aggregate-pinning test in
`nucleus/nucleus-compiler/tests/contract.rs` pins exactly this behaviour for
this example's `kernels.rs`.

## End-to-end status (HONEST)

- **`naive` × `pthreads-sync`** — passes bit-identically against
  `reference.bin`. Carries the correctness gate for this example.
  Wired into `nuc-nucleus/e2e-matrix.toml` and into
  `nucleus/nucleus-compiler/tests/e2e_example_07.rs`.

- **`blocked` × `pthreads-sync`** — `#[ignore]`'d, with a TODO
  reference to TASK-0143 (per-tile transfer hoisting). The schedule
  asks `block=8` on both `i` and `j`; N=16 is divisible by 8, so
  the block-transform pass accepts both rewrites. The reason the
  cell stays gated is downstream: example 7's transfers under a
  blocked schedule should fire per-tile, not per-iteration —
  TASK-0143 is the hoisting work. Until then the schedule is
  exercised by the parser / sched_lower / link pinning tests only.

## Required schedules

- `naive.sched.nuc` — single worker (`host`); the load-bearing
  correctness gate at M2.
- `blocked.sched.nuc` — single worker (`host`) with `loop i :
  block=8; loop j : block=8;`. Exercises the block-transform pass
  structurally (in both axes); full e2e is gated on TASK-0143.

## Honest limitations

1. **`wrapping_mul` + `wrapping_add`** mean overflow silently wraps
   rather than panics. The committed input fits comfortably; the
   choice is defensive for pathological inputs. PRD §10.1 demands
   bit-determinism, which wrapping arithmetic preserves and a
   `checked_*` chain would not (panic site dependent on schedule
   order).

2. **No distributed schedule yet.** PRD §9 row 7 calls out
   "all-to-all communication" — that's a property of a future
   `distributed.sched.nuc` (gated on TASK-0117 + transfer synthesis
   that understands all-to-all access). The algorithm is shaped to
   make that future schedule slot in cleanly (the three iter vars
   `i`, `j`, `k` are exposed by name; A's row `i`, B's column `j`,
   and C's cell `(i, j)` are derivable from the kernel's access
   pattern). But the schedule is not in this commit.

3. **Blocked e2e is `#[ignore]`'d** pending TASK-0143. Naive is the
   correctness gate for this milestone.

4. **N=16 is small.** Picked for CI speed and `xxd`-by-hand
   inspectability. A "real" matmul would be 256x256 or 1024x1024;
   that's a separate benchmark suite, not a correctness example.

5. **Single source of truth violation on N.** `prog.algo.nuc` and
   `kernels.rs` and `reference/src/main.rs` all carry `const N =
   16`. Same TASK-0103 dependency as examples 01 / 02 / 03 / 05.
