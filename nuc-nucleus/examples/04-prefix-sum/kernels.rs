// Kernel bodies for example 04-prefix-sum.
//
// PRD §6.2.2: kernel bodies are real Rust functions in an adjacent
// file, compiled by the host toolchain unmodified. Nucleus does NOT
// interpolate text into them.
//
// Five kernels:
//   - `accumulate(acc, x)`             — Pass-1 reduction fold.
//   - `exclusive_add(acc, x, b, c)`    — Pass-2 exclusive prefix step.
//   - `block_scan(acc, x, boff, i, j)` — Pass-3 within-block inclusive
//                                        scan + block offset.
//   - `load_input()`  — () -> Vec<i32>, effectful. Reads N i32 LE
//                        words from `input.bin` (or `$NUC_INPUT_PATH`).
//   - `save_output(v)`— (Vec<i32>) -> (), effectful. Writes N i32 LE
//                        words to `output.bin` (or `$NUC_OUTPUT_PATH`).
//
// Why the masking lives here and not in the algorithm
// ----------------------------------------------------
// Nuc v2 has no conditionals (PRD §6.2.4) and no boundary-safe index
// form: the textbook in-array carry `out[i] = out[i-1] + in[i]`
// panics at i=0 (usize underflow) and cannot be split base-case +
// loop (single-assignment is per data SYMBOL). Loop bounds must be
// const so a triangular `for j : 0..i+1` is rejected too. See
// `prog.algo.nuc`'s header and TASK-0039 / TASK-0179. The fix that
// stays in-language: keep the algorithm a rectangular
// reduction-accumulator (proven bit-identical by example 03 on both
// tier-1 backends) and put the "which terms contribute" predicate in
// these Rust kernels — which IS the intended division of labour
// (kernels say *what arithmetic*, the algorithm says *dataflow*).
//
// Why `Vec<i32>` and not `[i32; N]`
// ---------------------------------
// Same reason as examples 01/02/03/05: the PRD const-in-Rust-generics
// flow is unresolved (TASK-0103). `Vec<i32>` carries length at
// runtime; we check it explicitly. Trade-off: shape mismatch is a
// runtime panic, not a compile error. Resolves when TASK-0103 picks a
// convention.
//
// The algorithm declares `in_arr / out : i32[NB][BS]`. On the Rust
// side these are single flat `Vec<i32>` of length N = NB*BS = 256,
// row-major. The codegen flattens `in_arr[b][i]` to
// `in_arr[b*BS + i]` at compile time from the data shape;
// `load_input` returns the flat vector in file order, which MUST
// match that row-major layout. The committed `input.bin` (see README)
// is generated row-major by construction (block b's BS words
// contiguous).
//
// Contract pass (TASK-0012) expected behaviour:
//   - PASS for `accumulate`, `exclusive_add`, `block_scan` — all
//     scalar `(i32, ...) -> i32`.
//   - PASS for `save_output` viewed as scalar-in? No: it is declared
//     aggregate `i32[NB][BS]` -> (); together with `load_input`'s
//     aggregate return it surfaces the known `TypeMismatch`
//     (aggregate matching not yet implemented). Loud failure, pinned
//     by the contract test, identical in spirit to examples 03/05.
//
// I/O paths: same convention as the other examples — read
// `NUC_INPUT_PATH` / `NUC_OUTPUT_PATH` if set, else sibling filenames
// in cwd.

use std::env;
use std::fs;
use std::io::Write;

/// Length used by the algorithm. Mirrors `const N : usize = 256;` in
/// `prog.algo.nuc`. Single-source-of-truth violation (TASK-0103);
/// disappears when the const-flow convention is picked.
const N: usize = 256;

/// Pass-1 reduction fold step. Same shape/semantics as example 03's
/// `accumulate`. `wrapping_add` documents the overflow contract; the
/// committed fixture stays in-range (PRD §10.1 bit-determinism).
pub fn accumulate(acc: i32, x: i32) -> i32 {
    acc.wrapping_add(x)
}

/// Pass-2 exclusive-prefix step. Called for every (b, c) pair with
/// `acc = block_off[b]` (pre-init 0) and `x = block_sum[c]`. Adds the
/// block total only for blocks strictly BEFORE `b`, so after the full
/// `c : 0..NB` sweep `block_off[b] = sum_{c < b} block_sum[c]` — the
/// exclusive prefix. `c >= b` contributes nothing.
pub fn exclusive_add(acc: i32, x: i32, b: i32, c: i32) -> i32 {
    if c < b {
        acc.wrapping_add(x)
    } else {
        acc
    }
}

/// Pass-3 within-block inclusive scan + block offset. Called for
/// every (b, i, j) with `acc = out[b][i]` (pre-init 0),
/// `x = in_arr[b][j]`, `boff = block_off[b]`. Over the full
/// `j : 0..BS` sweep this accumulates:
///   - `in_arr[b][j]` for every j <= i  (the inclusive prefix of
///     block b up to position i), and
///   - `boff` exactly once (guarded by j == 0 so it is added a single
///     time regardless of i).
/// Result: `out[b][i] = block_off[b] + sum_{m=0..=i} in_arr[b][m]`,
/// i.e. the global inclusive prefix sum at flat index b*BS + i.
///
/// `j == 0` is always inside `0..BS` (BS = 64 > 0) so the offset is
/// added for every i; correctness does not depend on i ever being 0.
pub fn block_scan(acc: i32, x: i32, boff: i32, i: i32, j: i32) -> i32 {
    let mut v = acc;
    if j == 0 {
        v = v.wrapping_add(boff);
    }
    if j <= i {
        v = v.wrapping_add(x);
    }
    v
}

pub fn load_input() -> Vec<i32> {
    let path = env::var("NUC_INPUT_PATH").unwrap_or_else(|_| "input.bin".to_string());
    let bytes =
        fs::read(&path).unwrap_or_else(|e| panic!("load_input: cannot read {}: {}", path, e));
    let need = N * 4;
    assert!(
        bytes.len() >= need,
        "load_input: file {} has {} bytes; need at least {} (N={})",
        path,
        bytes.len(),
        need,
        N
    );
    let mut out = Vec::with_capacity(N);
    for i in 0..N {
        let off = i * 4;
        let word = i32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
        out.push(word);
    }
    out
}

pub fn save_output(v: Vec<i32>) {
    assert_eq!(
        v.len(),
        N,
        "save_output: expected {} elements (N = NB*BS), got {}",
        N,
        v.len()
    );
    let path = env::var("NUC_OUTPUT_PATH").unwrap_or_else(|_| "output.bin".to_string());
    let mut bytes = Vec::with_capacity(v.len() * 4);
    for x in &v {
        bytes.extend_from_slice(&x.to_le_bytes());
    }
    let mut f = fs::File::create(&path)
        .unwrap_or_else(|e| panic!("save_output: cannot create {}: {}", path, e));
    f.write_all(&bytes)
        .unwrap_or_else(|e| panic!("save_output: write failed: {}", e));
}
