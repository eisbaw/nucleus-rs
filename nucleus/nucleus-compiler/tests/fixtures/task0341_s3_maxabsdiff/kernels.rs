// Kernel bodies for the TASK-0341.02.01.04 (S3) max-abs-diff
// reduction fixture.
//
// Five kernels:
//   - `max_abs_acc(acc, n, o)` — scalar (i32, i32, i32) -> i32, pure.
//        The 3-arg per-element fold step: max(acc, abs(n - o)).
//        Mirrors 07-matmul's `madd(acc, x, y)` 3-arg accumulator
//        shape, with `+`/`*` replaced by `max`/`abs-of-diff`.
//   - `max_combine(a, b)`      — scalar (i32, i32) -> i32, pure.
//        The binary tree-reduction step: max(a, b). Mirrors
//        03-reduction's `combine(a, b)`.
//   - `load_new()` / `load_old()` — () -> Vec<i32>, effectful. Read
//        the two generations from a single input file (see below).
//   - `save_output(s)`         — (i32) -> (), effectful. Writes the
//        single i32 scalar as 4 LE bytes.
//
// `Vec<i32>` aggregate convention: per TASK-0103, aggregate-typed
// kernel signatures use `Vec<i32>` with a runtime length check.
//
// INPUT FILE LAYOUT. The fixture models a GENERATION PAIR, so a single
// `NUC_INPUT_PATH` file holds BOTH generations back-to-back:
//   bytes [0       .. N*4)   = `new`  (N=32 LE i32 words, row-major)
//   bytes [N*4     .. 2*N*4) = `old`  (N=32 LE i32 words, row-major)
// `load_new` returns the first N words; `load_old` returns the next N.
// Each is laid out row-major over `i32[NUM_WORKERS][PARTITION_SIZE]`.

use std::env;
use std::fs;
use std::io::Write;

/// Length of one generation. Mirrors `const N : usize = 32;` in
/// prog.algo.nuc (the doubled declaration is the v2 convention per
/// TASK-0103 — kernels.rs is plain Rust, no const text-substitution).
const N: usize = 32;

pub fn max_abs_acc(acc: i32, n: i32, o: i32) -> i32 {
    // Per-element L-infinity fold step. `(n - o).abs()` is the
    // per-element absolute difference; folding it with `max` over the
    // partition gives the per-partition max-abs-diff.
    //
    // OVERFLOW LIMIT (architect P2-1, cycle-259): `wrapping_sub` does NOT
    // make this overflow-safe — it only documents wraparound semantics on
    // the subtraction. The following `.abs()` STILL panics in debug (and
    // returns the negative `i32::MIN` in release, which would mis-rank the
    // max fold and BREAK the "abs>=0 ⇒ 0 is the max-identity" invariant)
    // when `n.wrapping_sub(o) == i32::MIN` (reachable e.g. n=0,
    // o=i32::MIN). This fixture is SOUND only because its committed input
    // is bounded (operands in [-500,-97], max abs-diff 186) — nowhere near
    // overflow. S5/.06 wires this SAME shape against REAL, unbounded
    // 16-jacobi generation data: that variant MUST use `unsigned_abs()`
    // (→ u32, widen/compare) or i64-widening before `.abs()`. Tracked as
    // TASK-0436; do NOT copy this kernel verbatim into S5.
    //
    // Modulo that bound: `.abs()` is always >= 0, so the zero-initialised
    // `acc` is the correct max-identity (max(0, nonneg) == nonneg) — the
    // crux that lets this reduction reuse the existing zero-init pre-init
    // pass with NO `init=` clause.
    acc.max(n.wrapping_sub(o).abs())
}

pub fn max_combine(a: i32, b: i32) -> i32 {
    // Binary tree-reduction step. `max` is associative, commutative,
    // and exact on i32 — so the scalar result is independent of the
    // fold order (pinned by task0341_s3_order_independence.rs). (This
    // epic is integer-only per PRD §10.1; IEEE-754 FP max has NaN /
    // signed-zero subtleties that are out of scope here.)
    a.max(b)
}

fn read_input_words() -> Vec<i32> {
    let path = env::var("NUC_INPUT_PATH").unwrap_or_else(|_| "input.bin".to_string());
    let bytes =
        fs::read(&path).unwrap_or_else(|e| panic!("load: cannot read {}: {}", path, e));
    let need = 2 * N * 4;
    assert!(
        bytes.len() >= need,
        "load: file {} has {} bytes; need at least {} (two generations of N={})",
        path,
        bytes.len(),
        need,
        N
    );
    let mut out = Vec::with_capacity(2 * N);
    for i in 0..(2 * N) {
        let off = i * 4;
        let word = i32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
        out.push(word);
    }
    out
}

pub fn load_new() -> Vec<i32> {
    read_input_words()[0..N].to_vec()
}

pub fn load_old() -> Vec<i32> {
    read_input_words()[N..(2 * N)].to_vec()
}

pub fn save_output(s: i32) {
    let path = env::var("NUC_OUTPUT_PATH").unwrap_or_else(|_| "output.bin".to_string());
    let bytes = s.to_le_bytes();
    let mut f = fs::File::create(&path)
        .unwrap_or_else(|e| panic!("save_output: cannot create {}: {}", path, e));
    f.write_all(&bytes)
        .unwrap_or_else(|e| panic!("save_output: write failed: {}", e));
}
