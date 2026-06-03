# Example 20 — index-cast permute (bare iter-var arg to an i32 index kernel)

The dedicated build-clean witness for **TASK-0431**: a PURE kernel called
in array-subscript **index** position with a **bare iteration-variable
argument**, which forces the sidecar-driven `(i) as i32` cast in the
generated crate.

## Why this example exists

`08-histogram/textbook` (TASK-0430) opened the X1' codegen path — a pure
kernel call in array-subscript **index** position:

```
histogram[bucket(input[i])] <-- inc(histogram[bucket(input[i])]);
```

But every previously-shipped cell on that path (`08-histogram/textbook`
and `scatter` + their `distributed` variants, `19-histogram-unconstrained`)
calls the index kernel with a **gather load** argument — `bucket(input[i])`,
where `input[i]` is already `i32`. None exercises a **bare iter var**
argument.

A bare iter var (`i`) renders **`i64`** in the generated host source (loop
counters are `i64`). Passed to an `i32`-param index kernel **without** a
per-param cast, the generated crate would hit `E0308` at build. rustc
catches it loudly (not a silent miscompile), but it is a usability footgun
in the exact path TASK-0430 opened. TASK-0431 adds the sidecar-driven
`(arg) as <ty>` cast to the shared `render_int_expr` `IrExpr::Call` arm,
mirroring `render_fire_arg`.

This example is the e2e build-clean witness. Its index expression emits as:

```rust
out[(kernels::idx((i) as i32)) as usize] = kernels::pass(src[(i) as usize]);
```

The `(i) as i32` is the TASK-0431 cast under test. Every tier-1 backend's
generated crate must compile and produce output bit-identical to
`reference.bin`.

## The algorithm

A deliberately trivial **identity permutation**:

```
out[idx(i)] <-- pass(src[i]);
```

with `idx(i) = i` (identity bijection over `0..N`) and `pass(x) = x`
(identity passthrough — the `15-transpose` `xpose` precedent for a pure
permutation; a kernel-less RHS is rejected by `acfg::build::build_dataflow`,
TASK-0360). So `out` is a verbatim copy of `src`.

The point is **not** the arithmetic — it is the **shape**: a pure kernel
`idx` called in index position with a bare iter-var `i`. The oracle is
therefore a plain copy.

### Honest limit on oracle strength

Because `idx`/`pass` are the identity, `reference.bin` equals `input.bin`
(a copy). A hypothetical backend that simply copied input → output without
evaluating `idx`/`pass` would also pass the byte-match. The **load-bearing**
assertion of this example is therefore the AC#1 acceptance criterion of
TASK-0431 — that the generated crate **builds clean** with the `(i) as i32`
cast across all tier-1 backends (a missing cast is a hard `E0308` build
failure, which the e2e harness — it compiles each emitted crate — turns
into a cell FAIL). The render-layer unit tests in
`nucleus/backend-common/tests/render_guard_siblings.rs`
(`int_expr_call_in_index_casts_iter_var_arg_to_i32_param` et al.) pin the
exact emitted cast string directly. The two together — render-string pin +
build-clean-across-backends e2e — are the complete TASK-0431 proof.

## Soundness

`idx(i) = i` is a bijection over `0..N`, so each `out` slot is written
exactly once (PRD §6.2.1 single-assignment). `idx` and `pass` are pure
(deterministic, side-effect-free), so they are sound in index / value
position (PRD §6.2). `out` and `src` are distinct data symbols (no
read/write aliasing).

> **Naming note.** The input array is named `src`, **not** `in`, because
> `in` is a reserved Rust keyword: the generated host source declares the
> array as a Rust `let`, so a data symbol named `in` would emit
> `let mut in = ...` and fail to compile. (Caught empirically while
> landing TASK-0431 — a general data-symbol/Rust-keyword collision, not
> specific to this example.)

## Files

```
prog.algo.nuc               # data src/out; out[idx(i)] <-- pass(src[i]) identity permute
kernels.rs                  # idx / pass / load_input / save_output (self-contained)
schedules/naive.sched.nuc   # single-worker (host)
input.bin                   # 1024 bytes — 256 i32 LE words
reference.bin               # 1024 bytes — 256 i32 LE words (== input.bin, identity copy)
reference/                  # independent std-only reference impl (direct copy, no idx/pass)
```

## Fixture (`input.bin`)

256 i32 LE words, generated deterministically by a Knuth multiplicative
hash (`v[i] = signed32((i * 2654435761) ^ ((i*2654435761) >> 13))`) so the
sequence is non-trivial and includes negatives (128 of 256). The exact
values do not matter for an identity permutation — only that the fixture is
reproducible. Regenerate with:

```python
import struct
N = 256
vals = []
for i in range(N):
    h = (i * 2654435761) & 0xFFFFFFFF
    h ^= (h >> 13)
    h &= 0xFFFFFFFF
    if h >= 0x80000000:
        h -= 0x100000000
    vals.append(h)
open('input.bin', 'wb').write(b''.join(struct.pack('<i', v) for v in vals))
```

Regenerate `reference.bin` (also via `just regen-references`):

```
cargo run --release \
  --manifest-path nuc-nucleus/examples/20-index-cast-permute/reference/Cargo.toml -- \
  --in    nuc-nucleus/examples/20-index-cast-permute/input.bin \
  --out   nuc-nucleus/examples/20-index-cast-permute/reference.bin
```

## Schedule

Single-worker `naive` only. A distributed variant would partition the `i`
loop, but the cast shape under test (`kernels::idx((i) as i32)`) is rendered
by the shared `render_int_expr` and is identical in single- and multi-worker
codegen, so the single-worker cell is the minimal sufficient witness.
