// Kernel bodies for example 23-dot-product (map-reduce / inner product).
//
// PRD §6.2.2: kernel bodies live in an adjacent Rust source file and
// are real Rust functions. Nucleus does not interpolate text into
// these bodies; they are compiled by the host toolchain unmodified.
//
// Six kernels:
//   - `mul(a, b)`          — scalar (i32, i32) -> i32, pure. The
//                            elementwise MAP step (phase 0).
//   - `accumulate(acc, x)` — scalar (i32, i32) -> i32, pure. The
//                            per-element fold step (phase 1). Byte-for
//                            -byte the same body as 03-reduction's.
//   - `combine(a, b)`      — scalar (i32, i32) -> i32, pure. The
//                            binary tree-reduction step (phase 2).
//   - `load_input()`       — () -> Vec<i32>, effectful. Reads N i32 LE
//                            words from input.bin positions [0..N).
//   - `load_input_b()`     — () -> Vec<i32>, effectful. Reads N i32 LE
//                            words from input.bin positions [N..2N).
//   - `save_output(s)`     — (i32) -> (), effectful. Writes the single
//                            i32 result as 4 LE bytes.
//
// Why `Vec<i32>` and not `[i32; N]`
// ---------------------------------
// Per TASK-0103 (Done cycle 17): `Vec<i32>` + runtime length check IS
// the canonical convention for aggregate-typed kernel signatures. The
// algorithm declares `a : i32[NUM_WORKERS][PARTITION_SIZE]`. On the
// Rust side this is a single flat `Vec<i32>` of length N =
// NUM_WORKERS * PARTITION_SIZE = 256, laid out row-major. The codegen
// flattens the 2D index `a[w][i]` to `a[w * PARTITION_SIZE + i]` at
// compile time using the data shape. `load_input` / `load_input_b`
// return their flat vectors in file order — that file order MUST match
// the row-major layout (partition w's bytes contiguous). `input.bin`'s
// generator (see reference/) writes both vectors in that order by
// construction: a in [0..N), b in [N..2N).
//
// Contract pass (TASK-0012) expected behaviour:
//   - PASS for `mul`, `accumulate`, `combine`  — all `(i32, i32) -> i32`.
//   - PASS for `save_output`                   — scalar `(i32) -> ()`.
//   - `TypeMismatch` for `load_input` / `load_input_b` — Nuc-declared
//     aggregate (`i32[NUM_WORKERS][PARTITION_SIZE]`) matched against
//     `Vec<i32>`; scalar-only matching at M1 (loud failure, not a
//     bug in this example).
//
// I/O paths: same convention as examples 01 / 02 / 03. Read from
// `NUC_INPUT_PATH` / `NUC_OUTPUT_PATH` if set, else conventional
// sibling filenames in the cwd.

use std::env;
use std::fs;
use std::io::Write;

/// Length used by the algorithm. Mirrors `const N : usize = 256;` in
/// `prog.algo.nuc`. The doubled declaration is the v2 convention per
/// TASK-0103 (Done cycle 17): kernels.rs is plain Rust compiled by the
/// host toolchain unmodified — Nucleus does not text-substitute
/// algorithm consts into kernel bodies.
const N: usize = 256;

pub fn mul(a: i32, b: i32) -> i32 {
    // Wrapping multiply — deterministic on overflow (two's complement
    // wraparound). The committed fixture (see reference/) keeps every
    // product well inside the i32 range, but the contract is explicit.
    // PRD §10.1: integer arithmetic is bit-deterministic; `wrapping_mul`
    // ensures even pathological inputs neither panic in debug nor
    // invoke undefined behaviour in release.
    a.wrapping_mul(b)
}

pub fn accumulate(acc: i32, x: i32) -> i32 {
    // Wrapping add — the per-element fold step. Same body as
    // 03-reduction's `accumulate`. `wrapping_add` is associative and
    // commutative over i32, so the fold (and the later tree combine)
    // is reorder-invariant — the basis for the cross-backend
    // bit-identity (PRD §10.1).
    acc.wrapping_add(x)
}

pub fn combine(a: i32, b: i32) -> i32 {
    // The binary tree-reduction step. Same semantics as `accumulate`
    // for a sum reduction, but kept as a separate kernel so schedules
    // and Petri-net analyses can distinguish phase-1 firings from
    // phase-2 firings (today both lower to the same Rust expression).
    a.wrapping_add(b)
}

pub fn load_input() -> Vec<i32> {
    let path = env::var("NUC_INPUT_PATH").unwrap_or_else(|_| "input.bin".to_string());
    read_i32_le_slice(&path, 0, N)
}

pub fn load_input_b() -> Vec<i32> {
    let path = env::var("NUC_INPUT_PATH").unwrap_or_else(|_| "input.bin".to_string());
    read_i32_le_slice(&path, N, N)
}

pub fn save_output(s: i32) {
    let path = env::var("NUC_OUTPUT_PATH").unwrap_or_else(|_| "output.bin".to_string());
    let bytes = s.to_le_bytes();
    let mut f = fs::File::create(&path)
        .unwrap_or_else(|e| panic!("save_output: cannot create {}: {}", path, e));
    f.write_all(&bytes)
        .unwrap_or_else(|e| panic!("save_output: write failed: {}", e));
}

/// Read `count` little-endian i32 words from `path`, starting at
/// element offset `start` (i.e. byte offset `start * 4`).
fn read_i32_le_slice(path: &str, start: usize, count: usize) -> Vec<i32> {
    let bytes =
        fs::read(path).unwrap_or_else(|e| panic!("load_input: cannot read {}: {}", path, e));
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
        let word = i32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
        out.push(word);
    }
    out
}
