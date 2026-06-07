// Kernel bodies for example 29-jacobi-cap-hit. Landed TASK-0453.05
// (rigour epic P5). The CAP-HIT (did-NOT-converge) sibling of
// 21-jacobi-converge.
//
// The kernel bodies are BYTE-IDENTICAL to 21-jacobi-converge's: the
// step semantics, the L-infinity fold, and the IO are unchanged. Only
// the algorithm's `ITERS_CAP` const differs (16 here vs 64 there), so
// the `until` predicate never fires inside the cap and the loop runs
// the full worst-case replay. See `prog.algo.nuc` for the cap-hit
// rationale.
//
// PRD §6.2.2: kernel bodies live in an adjacent Rust source file and
// are real Rust functions compiled by the host toolchain unmodified.
//
// Five kernels:
//   - `jacobi5_or_seed(pn, ps, pe, pw, sy, t)` — scalar
//                     `(i32, i32, i32, i32, i32, i32) -> i32`, pure.
//                     Returns `sy` when `t == 0`; returns
//                     `(pn + ps + pe + pw) / 4` (truncating) when
//                     `t >= 1`. IDENTICAL to 16-jacobi / 21-jacobi-converge.
//   - `max_abs_acc(acc, n, o)` — scalar (i32, i32, i32) -> i32, pure.
//                     The 3-arg per-element L-infinity fold step:
//                     `max(acc, |n - o|)`. OVERFLOW-SAFE per TASK-0436.
//   - `ident(v)`    — scalar (i32) -> i32, pure. Identity copy. Stages
//                     the accumulator into `maxdiff[t]` and the
//                     last-computed generation into `result`.
//   - `load_input`  — () -> Vec<i32>, effectful. Reads H*W i32 LE words
//                     from `input.bin` (row-major).
//   - `save_output` — (Vec<i32>) -> (), effectful. Writes H*W i32 LE
//                     words to the output path (row-major).
//
// OVERFLOW-SAFE absolute difference (TASK-0436)
// ---------------------------------------------
// `abs_diff_i32` widens both operands to i64 (so the subtraction never
// overflows), takes the unsigned magnitude (`unsigned_abs`; no
// i32::MIN.abs() panic path), then clamps to `i32::MAX`. The reference
// oracle uses the IDENTICAL expression so the differential stays
// meaningful. See 21-jacobi-converge's kernels.rs header for the full
// determinism / clamp rationale.
//
// Numeric type choice: i32 with truncating division (as 16-jacobi). The
// committed seed (shared with 21-jacobi-converge) keeps every interior
// cell in 0..256; `maxdiff[t]` first drops to <= TOL=2 at generation 30
// — strictly GREATER than this example's cap of 16, so the loop runs the
// full `0..ITERS_CAP+1` worst case and never converges inside the cap.

use std::env;
use std::fs;
use std::io::Write;

/// Dimensions used by the algorithm. Mirror `const H : usize = 8;`,
/// `const W : usize = 8;` in `prog.algo.nuc` (TASK-0103; Nucleus does
/// not text-substitute algorithm consts into kernel bodies).
const H: usize = 8;
const W: usize = 8;
const N: usize = H * W;

/// Overflow-safe absolute difference of two i32 values, clamped to
/// `i32::MAX` (TASK-0436). Used by both this kernel and the reference
/// oracle — they MUST agree bit-for-bit.
fn abs_diff_i32(n: i32, o: i32) -> i32 {
    let mag: u64 = (i64::from(n) - i64::from(o)).unsigned_abs();
    mag.min(i32::MAX as u64) as i32
}

/// One Jacobi-step kernel call, seed-fallback on `t`. Identical to
/// 16-jacobi / 21-jacobi-converge's `jacobi5_or_seed`.
pub fn jacobi5_or_seed(
    prev_n: i32,
    prev_s: i32,
    prev_e: i32,
    prev_w: i32,
    seed_yx: i32,
    t: i32,
) -> i32 {
    if t == 0 {
        seed_yx
    } else {
        let sum = prev_n
            .wrapping_add(prev_s)
            .wrapping_add(prev_e)
            .wrapping_add(prev_w);
        sum / 4
    }
}

/// 3-arg pure L-infinity accumulator: `max(acc, |n - o|)`. The
/// `|n - o|` is overflow-safe (see `abs_diff_i32`). Zero-init of the
/// accumulator slot is the correct max-identity (every term >= 0).
pub fn max_abs_acc(acc: i32, n: i32, o: i32) -> i32 {
    acc.max(abs_diff_i32(n, o))
}

/// Identity copy.
pub fn ident(v: i32) -> i32 {
    v
}

pub fn load_input() -> Vec<i32> {
    let path = env::var("NUC_INPUT_PATH").unwrap_or_else(|_| "input.bin".to_string());
    let bytes =
        fs::read(&path).unwrap_or_else(|e| panic!("load_input: cannot read {}: {}", path, e));
    let need = N * 4;
    assert!(
        bytes.len() >= need,
        "load_input: file {} has {} bytes; need at least {} (H*W = {}*{} = {})",
        path,
        bytes.len(),
        need,
        H,
        W,
        N
    );
    let mut out = Vec::with_capacity(N);
    for i in 0..N {
        let off = i * 4;
        let word = i32::from_le_bytes([
            bytes[off],
            bytes[off + 1],
            bytes[off + 2],
            bytes[off + 3],
        ]);
        out.push(word);
    }
    out
}

pub fn save_output(data: Vec<i32>) {
    assert_eq!(
        data.len(),
        N,
        "save_output: expected {} elements (H*W = {}*{}), got {}",
        N,
        H,
        W,
        data.len()
    );
    let path = env::var("NUC_OUTPUT_PATH").unwrap_or_else(|_| "output.bin".to_string());
    let mut bytes = Vec::with_capacity(data.len() * 4);
    for v in &data {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    let mut f = fs::File::create(&path)
        .unwrap_or_else(|e| panic!("save_output: cannot create {}: {}", path, e));
    f.write_all(&bytes)
        .unwrap_or_else(|e| panic!("save_output: write failed: {}", e));
}
