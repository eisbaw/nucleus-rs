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

A **reversal permutation**:

```
out[idx(i)] <-- pass(src[i]);
```

with `idx(i) = N-1-i` (reversal bijection over `0..N`) and `pass(x) = x`
(identity passthrough — the `15-transpose` `xpose` precedent for a pure
permutation; a kernel-less RHS is rejected by `acfg::build::build_dataflow`,
TASK-0360). So `out` is `src` **reversed**.

The point is **not** the arithmetic — it is the **shape**: a pure kernel
`idx` called in index position with a bare iter-var `i`.

### Oracle strength

The reversal (vs an identity) is deliberate (TASK-0431.01). With an identity
`idx`, `reference.bin` would equal `input.bin`, and a backend that merely
copied input → output without evaluating `idx`/`pass` would also pass the
byte-match — a weak oracle. The reversal makes `reference.bin` the input
**reversed**, so the e2e byte-match is now **value-discriminating**: a
backend that emitted the `(i) as i32` cast but mis-evaluated the index — or
skipped `idx` entirely — mismatches the reference.

This complements, it does not replace, the other two TASK-0431 proofs: the
generated crate must still **build clean** with the `(i) as i32` cast across
all tier-1 backends (a missing cast is a hard `E0308` build failure, which
the e2e harness — it compiles each emitted crate — turns into a cell FAIL),
and the render-layer unit tests in
`nucleus/backend-common/tests/render_guard_siblings.rs`
(`int_expr_call_in_index_casts_iter_var_arg_to_i32_param` et al.) pin the
exact emitted cast string directly. Render-string pin +
build-clean-across-backends + value-discriminating reversal oracle are the
complete TASK-0431 proof.

## Soundness

`idx(i) = N-1-i` is a bijection over `0..N`, so each `out` slot is written
exactly once (PRD §6.2.1 single-assignment). `idx` and `pass` are pure
(deterministic, side-effect-free), so they are sound in index / value
position (PRD §6.2). `out` and `src` are distinct data symbols (no
read/write aliasing).

> **Naming note.** The input array is named `src`, **not** `in`, because
> `in` is a reserved Rust keyword. Such an identifier is now rejected
> fail-loud at the front-end with a source-site diagnostic (TASK-0433);
> before that guard landed it slipped through to the generated host source
> as `let mut in = ...` and failed to compile. (Caught empirically while
> landing TASK-0431 — a general data-symbol/Rust-keyword collision, not
> specific to this example; the reject is the TASK-0433 fix.)

## Files

```
prog.algo.nuc               # data src/out; out[idx(i)] <-- pass(src[i]) reversal permute
kernels.rs                  # idx / pass / load_input / save_output (self-contained)
schedules/naive.sched.nuc   # single-worker (host)
input.bin                   # 1024 bytes — 256 i32 LE words
reference.bin               # 1024 bytes — 256 i32 LE words (== input.bin reversed)
reference/                  # independent std-only reference impl (direct reversal, no idx/pass)
```

## Fixture (`input.bin`)

256 i32 LE words, generated deterministically by a Knuth multiplicative
hash (`v[i] = signed32((i * 2654435761) ^ ((i*2654435761) >> 13))`) so the
sequence is non-trivial and includes negatives (128 of 256). The exact
values do not matter for a permutation — only that the fixture is
reproducible. (The reversal oracle additionally requires the sequence be
non-palindromic, which this hash trivially satisfies, so reversed ≠ input
and the discriminating power is real.) Regenerate with:

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
