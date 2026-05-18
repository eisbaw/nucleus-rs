// Kernel bodies for example 07-matmul.
//
// PRD §6.2.2: kernel bodies live in an adjacent Rust source file and
// are real Rust functions. Nucleus does NOT interpolate text into
// these bodies; they are compiled by the host toolchain unmodified.
//
// Four kernels:
//   - `madd(acc, x, y)` — scalar (i32, i32, i32) -> i32, pure.
//                         Multiply-accumulate: acc + x * y.
//   - `load_a()`        — () -> Vec<i32>, effectful. Reads N*N i32
//                         LE words for matrix A from the first half
//                         of `input.bin` (or `$NUC_INPUT_PATH`).
//   - `load_b()`        — () -> Vec<i32>, effectful. Reads matrix B
//                         from the second half.
//   - `save_c(c)`       — (Vec<i32>) -> (), effectful. Writes N*N
//                         i32 LE words to `output.bin` (or
//                         `$NUC_OUTPUT_PATH`).
//
// Why `Vec<i32>` and not `[i32; N*N]`
// -----------------------------------
// Same reason as examples 01 / 02 / 03 / 05: PRD const-in-Rust-
// generics flow is unresolved (TASK-0103). `Vec<i32>` carries
// length at runtime; we check it explicitly in `save_c`. Trade-off:
// shape errors become runtime panics rather than compile-time
// mismatches. Resolves when TASK-0103 picks a convention.
//
// The algorithm declares `a, b, c : i32[N][N]`. On the Rust side
// these are single flat `Vec<i32>` of length N*N = 256, laid out
// row-major. The codegen flattens the 2D index `c[i][j]` to
// `c[i * N + j]` at compile time using the data shape.
//
// Input file layout (`input.bin`, 2048 bytes total):
//   bytes [0    .. 1024) — A, row-major, N*N i32 LE words.
//   bytes [1024 .. 2048) — B, row-major, N*N i32 LE words.
//
// Output file layout (`output.bin`, 1024 bytes):
//   bytes [0 .. 1024) — C, row-major, N*N i32 LE words.
//
// `load_a` and `load_b` both read the SAME input file; each slices
// its half. We split this way (rather than one `load_input` that
// returns the pair) to keep the kernel signatures simple and match
// the PRD §6.2.2 sketch shape — one kernel per logical input.
//
// Why integer multiply-accumulate
// -------------------------------
// PRD §10.1 wants bit-deterministic output across schedules and
// backends. Integer arithmetic is bit-deterministic; floating-point
// multiply-accumulate ordering is NOT (a sum of products of floats
// is reorderable, and schedules reorder reductions). i32 fits all
// realistic matmul sizes the example needs, with the input pattern
// chosen to stay inside i32 range (see overflow note on `madd`).
//
// Contract pass (TASK-0012) expected behaviour against this file:
//   - PASS for `madd`              — declared `(i32, i32, i32) ->
//                                     i32`, scalar params/return.
//   - `TypeMismatch` for `load_a`, `load_b`, `save_c` — declared
//                                     aggregate `i32[N][N]`, matched
//                                     against `Vec<i32>` (scalar-
//                                     only matching at M1). Loud
//                                     failure, pinned by the
//                                     contract test.
//
// I/O paths: same convention as examples 01 / 02 / 03 / 05. Read
// from `NUC_INPUT_PATH` / `NUC_OUTPUT_PATH` if set, else
// conventional sibling filenames in the cwd.

use std::env;
use std::fs;
use std::io::Write;

/// Matrix dimension. Mirrors `const N : usize = 16;` in
/// `prog.algo.nuc`. Single-source-of-truth violation (TASK-0103);
/// disappears when the const-flow convention is picked.
const N: usize = 16;
const ELEMS: usize = N * N;
const BYTES_PER_WORD: usize = 4;
const MATRIX_BYTES: usize = ELEMS * BYTES_PER_WORD;
const INPUT_BYTES: usize = 2 * MATRIX_BYTES;

/// Scalar multiply-accumulate: `acc + x * y`.
///
/// Overflow note: with the committed input pattern (see README) each
/// element is in -6..=6, so each k-step adds at most 36 (in
/// magnitude) and a row's worth of N=16 steps sums to at most ~576,
/// well inside i32. `wrapping_mul` and `wrapping_add` document the
/// overflow contract — pathological inputs do not panic.
pub fn madd(acc: i32, x: i32, y: i32) -> i32 {
    acc.wrapping_add(x.wrapping_mul(y))
}

/// Read matrix A from the first half of the input file.
pub fn load_a() -> Vec<i32> {
    let bytes = read_input_bin();
    decode_matrix(&bytes, 0)
}

/// Read matrix B from the second half of the input file.
pub fn load_b() -> Vec<i32> {
    let bytes = read_input_bin();
    decode_matrix(&bytes, MATRIX_BYTES)
}

/// Read the whole input.bin (2048 bytes) and validate length.
fn read_input_bin() -> Vec<u8> {
    let path = env::var("NUC_INPUT_PATH").unwrap_or_else(|_| "input.bin".to_string());
    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("load: cannot read {}: {}", path, e));
    assert!(
        bytes.len() >= INPUT_BYTES,
        "load: file {} has {} bytes; need at least {} (2 * N*N*4, N={})",
        path,
        bytes.len(),
        INPUT_BYTES,
        N
    );
    bytes
}

fn decode_matrix(bytes: &[u8], offset: usize) -> Vec<i32> {
    let mut out = Vec::with_capacity(ELEMS);
    for k in 0..ELEMS {
        let off = offset + k * BYTES_PER_WORD;
        let word = i32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
        out.push(word);
    }
    out
}

pub fn save_c(c: Vec<i32>) {
    assert_eq!(
        c.len(),
        ELEMS,
        "save_c: expected {} elements (N*N = {}*{}), got {}",
        ELEMS,
        N,
        N,
        c.len()
    );
    let path = env::var("NUC_OUTPUT_PATH").unwrap_or_else(|_| "output.bin".to_string());
    let mut bytes = Vec::with_capacity(MATRIX_BYTES);
    for v in &c {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    let mut f =
        fs::File::create(&path).unwrap_or_else(|e| panic!("save_c: cannot create {}: {}", path, e));
    f.write_all(&bytes)
        .unwrap_or_else(|e| panic!("save_c: write failed: {}", e));
}
