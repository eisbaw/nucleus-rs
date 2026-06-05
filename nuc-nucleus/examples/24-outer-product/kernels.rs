// Kernel bodies for example 24-outer-product (rank-1 / outer product).
//
// PRD §6.2.2: kernel bodies live in an adjacent Rust source file and
// are real Rust functions. Nucleus does NOT interpolate text into these
// bodies; they are compiled by the host toolchain unmodified.
//
// Four kernels:
//   - `mul(x, y)`       — scalar (i32, i32) -> i32, pure. The single
//                         outer-product step: x * y. No accumulate.
//   - `load_a()`        — () -> Vec<i32>, effectful. Reads M i32 LE
//                         words for vector a from input.bin positions
//                         [0..M).
//   - `load_b()`        — () -> Vec<i32>, effectful. Reads N i32 LE
//                         words for vector b from positions [M..M+N).
//   - `save_output(c)`  — (Vec<i32>) -> (), effectful. Writes the M*N
//                         row-major flattened matrix c as M*N i32 LE
//                         words to output.bin.
//
// Why `Vec<i32>` and not `[i32; M]` / `[i32; M*N]`
// ------------------------------------------------
// Per TASK-0103 (Done cycle 17): `Vec<i32>` + runtime length check IS
// the canonical convention for aggregate-typed kernel signatures. The
// algorithm declares `a : i32[M]`, `b : i32[N]`, `c : i32[M][N]`. On
// the Rust side these are single flat `Vec<i32>`s laid out row-major.
// The codegen flattens the 2D index `c[i][j]` to `c[i * N + j]` at
// compile time using the data shape (identical to 07-matmul). The 1D
// reads `a[i]` / `b[j]` are already flat. `load_a` / `load_b` return
// their flat vectors in file order; that file order MUST match the
// algorithm's layout — `input.bin`'s generator (see reference/) writes
// a in [0..M) then b in [M..M+N) by construction.
//
// Input file layout (`input.bin`, (M+N)*4 = 96 bytes total):
//   bytes [0       .. 4*M)     — a, M i32 LE words.
//   bytes [4*M     .. 4*(M+N)) — b, N i32 LE words.
//
// Output file layout (`output.bin`, M*N*4 = 512 bytes):
//   bytes [0 .. 512) — c, row-major, M*N i32 LE words.
//
// Why integer multiply (no accumulate)
// -------------------------------------
// PRD §10.1 wants bit-deterministic output across schedules and
// backends. Integer multiply is bit-deterministic. Unlike a reduction,
// the outer product has NO fold to reorder, so bit-identity is
// automatic — there is not even an associativity/commutativity argument
// to make. `wrapping_mul` documents the overflow contract; the
// committed fixture (see README) keeps every product tiny.
//
// Contract pass (TASK-0012) expected behaviour against this file:
//   - PASS for `mul`            — declared `(i32, i32) -> i32`, scalar.
//   - `TypeMismatch` for `load_a` / `load_b` / `save_output` — declared
//     aggregate (`i32[M]`, `i32[N]`, `i32[M][N]`), matched against
//     `Vec<i32>`; scalar-only matching at M1 (loud failure, same shape
//     as examples 01/07/23, not a bug in this example).
//
// I/O paths: same convention as examples 01 / 07 / 23. Read from
// `NUC_INPUT_PATH` / `NUC_OUTPUT_PATH` if set, else conventional
// sibling filenames in the cwd.

use std::env;
use std::fs;
use std::io::Write;

/// Vector lengths. Mirror `const M : usize = 8;` and
/// `const N : usize = 16;` in `prog.algo.nuc`. The doubled declaration
/// is the v2 convention per TASK-0103 (Done cycle 17): kernels.rs is
/// plain Rust compiled by the host toolchain unmodified — Nucleus does
/// not text-substitute algorithm consts into kernel bodies.
const M: usize = 8;
const N: usize = 16;
const ELEMS_C: usize = M * N;

pub fn mul(x: i32, y: i32) -> i32 {
    // Wrapping multiply — the single outer-product step. Deterministic
    // on overflow (two's-complement wraparound). The committed fixture
    // (see reference/) keeps every product well inside the i32 range,
    // but the contract is explicit. PRD §10.1: integer arithmetic is
    // bit-deterministic; with no reduction the result is order-
    // independent by construction, so every backend emits identical
    // bytes.
    x.wrapping_mul(y)
}

pub fn load_a() -> Vec<i32> {
    let path = env::var("NUC_INPUT_PATH").unwrap_or_else(|_| "input.bin".to_string());
    read_i32_le_slice(&path, 0, M)
}

pub fn load_b() -> Vec<i32> {
    let path = env::var("NUC_INPUT_PATH").unwrap_or_else(|_| "input.bin".to_string());
    read_i32_le_slice(&path, M, N)
}

pub fn save_output(c: Vec<i32>) {
    assert_eq!(
        c.len(),
        ELEMS_C,
        "save_output: expected {} elements (M*N = {}*{}), got {}",
        ELEMS_C,
        M,
        N,
        c.len()
    );
    let path = env::var("NUC_OUTPUT_PATH").unwrap_or_else(|_| "output.bin".to_string());
    let mut bytes = Vec::with_capacity(ELEMS_C * 4);
    for v in &c {
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
    let bytes = fs::read(path).unwrap_or_else(|e| panic!("load: cannot read {}: {}", path, e));
    let need = (start + count) * 4;
    assert!(
        bytes.len() >= need,
        "load: file {} has {} bytes; need at least {}",
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
