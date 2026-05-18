// Kernel bodies for example 03-reduction.
//
// Four kernels:
//   - `accumulate(acc, x)`  — scalar (i32, i32) -> i32, pure. The
//                             per-element fold step used in phase 1.
//   - `combine(a, b)`       — scalar (i32, i32) -> i32, pure. The
//                             binary tree-reduction step used in
//                             phase 2.
//   - `load_input()`        — () -> Vec<i32>, effectful. Reads N
//                             i32 little-endian words from input.bin
//                             (or `$NUC_INPUT_PATH`).
//   - `save_output(s)`      — (i32) -> (), effectful. Writes the
//                             single i32 result as 4 LE bytes to
//                             output.bin (or `$NUC_OUTPUT_PATH`).
//
// Why `Vec<i32>` and not `[i32; N]`
// ---------------------------------
// Same reasoning as example 01/02: PRD const-in-Rust-generics flow
// is unresolved (TASK-0103). `Vec<i32>` carries length at runtime;
// we check it explicitly in `load_input` (and in the codegen on
// indexing). Trade-off: shape error becomes a runtime panic rather
// than a compile-time mismatch. Resolves when TASK-0103 picks a
// convention.
//
// The algorithm declares `a : i32[NUM_WORKERS][PARTITION_SIZE]`. On
// the Rust side this is a single flat `Vec<i32>` of length N =
// NUM_WORKERS * PARTITION_SIZE = 256, laid out row-major. The
// codegen flattens the 2D index `a[w][i]` to `a[w * PARTITION_SIZE +
// i]` at compile time using the data shape. `load_input` just
// returns the flat vector in file order — that file order MUST
// match the row-major layout (partition w's bytes contiguous).
// `input.bin`'s generator (see README) writes the file in that
// order by construction.
//
// Contract pass (TASK-0012) expected behaviour:
//   - PASS for `accumulate`, `combine`  — both `(i32, i32) -> i32`.
//   - PASS for `save_output`            — scalar `(i32) -> ()`.
//   - `TypeMismatch` for `load_input`   — Nuc-declared aggregate
//                                         (`i32[NUM_WORKERS][PARTITION_SIZE]`),
//                                         matched against `Vec<i32>`;
//                                         scalar-only matching at M1.
// The contract test pins this loud-failure mode (TASK-0103).
//
// I/O paths: same convention as examples 01 / 02. Read from
// `NUC_INPUT_PATH` / `NUC_OUTPUT_PATH` if set, else conventional
// sibling filenames in the cwd.

use std::env;
use std::fs;
use std::io::Write;

/// Length used by the algorithm. Mirrors `const N : usize = 256;` in
/// `prog.algo.nuc`. Single-source-of-truth violation (TASK-0103);
/// disappears when the const-flow convention is picked.
const N: usize = 256;

pub fn accumulate(acc: i32, x: i32) -> i32 {
    // Wrapping add — deterministic on overflow (two's complement
    // wraparound). The committed input pattern (see README) keeps
    // the running sum inside the i32 range, but the contract is
    // explicit. PRD §10.1: integer arithmetic is bit-deterministic;
    // `wrapping_add` ensures even pathological inputs do not panic
    // in debug nor invoke undefined behaviour in release.
    acc.wrapping_add(x)
}

pub fn combine(a: i32, b: i32) -> i32 {
    // The binary tree-reduction step. Same semantics as
    // `accumulate` for a sum reduction, but kept as a separate
    // kernel because schedules and Petri-net analyses may want to
    // distinguish phase 1 firings from phase 2 firings. (Today
    // both lower to the same Rust expression; if a future schedule
    // pipelines or distributes them differently, the distinction
    // pays off.)
    a.wrapping_add(b)
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

pub fn save_output(s: i32) {
    let path = env::var("NUC_OUTPUT_PATH").unwrap_or_else(|_| "output.bin".to_string());
    let bytes = s.to_le_bytes();
    let mut f = fs::File::create(&path)
        .unwrap_or_else(|e| panic!("save_output: cannot create {}: {}", path, e));
    f.write_all(&bytes)
        .unwrap_or_else(|e| panic!("save_output: write failed: {}", e));
}
