// Kernel bodies for example 09-producer-consumer.
//
// PRD §6.2.2: kernel bodies live in an adjacent Rust source file and
// are real Rust functions. Nucleus does NOT interpolate text into
// these bodies; they are compiled by the host toolchain unmodified.
//
// Four kernels:
//   - `produce`     — scalar (i32) -> i32, pure.
//                     `produce(s) = s.wrapping_mul(3)`. Stage-1 of the
//                     producer/consumer pipe.
//   - `transform`   — scalar (i32) -> i32, pure.
//                     `transform(r) = r.wrapping_mul(7).wrapping_add(r)`
//                     (i.e. `r * 8` in two-explicit-ops). Stage-2 of the
//                     pipe; the two-op shape is deliberately
//                     non-degenerate so a bug that drops `r` or swaps
//                     operands shows up in the output bits.
//   - `load_input`  — () -> Vec<i32>, effectful. Reads N i32 LE words
//                     from `input.bin` (positions 0..N).
//   - `save_output` — (Vec<i32>) -> (), effectful. Writes N i32 LE
//                     words to the output path.
//
// Numeric type choice: i32
// ------------------------
// PRD §13 and docs/reference-impl-policy.md §5. Integer arithmetic is
// bit-deterministic by Rust's language definition; floating-point
// reductions are not. This example performs no reduction, but staying
// i32 matches every other example and lets `wrapping_*` document the
// overflow contract explicitly. The committed input.bin (seeds
// 1..N=16) stays well inside the i32 range — none of the per-element
// operations come anywhere near `i32::MAX`.
//
// Why `Vec<i32>` and not `[i32; N]`
// ---------------------------------
// See examples 01..07. TASK-0103 is the open PRD question for
// aggregate-type matching; until it lands, aggregate kernel I/O uses
// `Vec<i32>` with a runtime length assertion in `save_output`.
//
// Contract pass (TASK-0012) behaviour expected against this file
// --------------------------------------------------------------
// `check_kernels_contract` is scalar-only at present. It will:
//   - PASS for `produce`   — declared `(i32) -> i32`, signature matches.
//   - PASS for `transform` — declared `(i32) -> i32`, signature matches.
//   - REPORT `TypeMismatch` with "aggregate type matching is not yet
//     implemented" for `load_input` and `save_output` because their
//     Nuc-side declarations are aggregate (`i32[N]`). Loud failure,
//     not silent acceptance — same pattern as every other example;
//     not a bug here.
//
// I/O paths
// ---------
// Read paths from environment variables when set, falling back to
// conventional sibling filenames in the cwd. This is what the
// pthreads-sync / pthreads-async / mp-tcp-bufsync emitted host
// program threads in via `NUC_INPUT_PATH` / `NUC_OUTPUT_PATH`.

use std::env;
use std::fs;
use std::io::Write;

/// Length used by the algorithm. Mirrors `const N : usize = 16;` in
/// `prog.algo.nuc`. The duplication is the single-source-of-truth
/// violation called out in TASK-0103 and shared with every other
/// example.
const N: usize = 16;

/// Stage-1 producer kernel. `produce(seed) = seed * 3`, with
/// `wrapping_mul` to make the overflow contract explicit (the
/// committed input never trips it).
pub fn produce(seed: i32) -> i32 {
    seed.wrapping_mul(3)
}

/// Stage-2 consumer kernel. `transform(rec) = rec * 7 + rec` (i.e.
/// `rec * 8`, written as two operations on purpose: the two-op shape
/// catches a bug that drops one operand or uses `+` instead of
/// `wrapping_add`).
pub fn transform(rec: i32) -> i32 {
    rec.wrapping_mul(7).wrapping_add(rec)
}

pub fn load_input() -> Vec<i32> {
    let path = env::var("NUC_INPUT_PATH").unwrap_or_else(|_| "input.bin".to_string());
    read_i32_le_slice(&path, 0, N)
}

pub fn save_output(data: Vec<i32>) {
    assert_eq!(
        data.len(),
        N,
        "save_output: expected {} elements, got {}",
        N,
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

/// Read `count` little-endian i32 words from `path`, starting at
/// element offset `start` (i.e. byte offset `start * 4`).
fn read_i32_le_slice(path: &str, start: usize, count: usize) -> Vec<i32> {
    let bytes = fs::read(path)
        .unwrap_or_else(|e| panic!("load_input: cannot read {}: {}", path, e));
    let need = (start + count) * 4;
    assert!(
        bytes.len() >= need,
        "load_input: file {} has {} bytes; need at least {}",
        path,
        bytes.len(),
        need
    );
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let off = (start + i) * 4;
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
