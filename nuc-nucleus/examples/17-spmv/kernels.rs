// Kernel bodies for example 17-spmv. Landed cycle 210 (TASK-0341.03
// AC#1 language-sanity slice).
//
// PRD §6.2.2: kernel bodies live in an adjacent Rust source file and
// are real Rust functions. Nucleus does NOT interpolate text into
// these bodies; they are compiled by the host toolchain unmodified.
//
// Five kernels:
//   - `spmv_step(acc, v, c, x_j, j)` — scalar (i32,i32,i32,i32,i32)
//                     -> i32, pure. Masked multiply-accumulate:
//                     returns `acc + v*x_j` iff `j == c`, else
//                     `acc`. The data-dependent indexing that the
//                     algorithm sublanguage cannot express (PRD
//                     §6.2.4: no conditionals; grammar §1: nested
//                     IndexSuffix not in IndexExpr.Atom) lives
//                     here in plain Rust.
//   - `load_val`     — () -> Vec<i32>, effectful. Reads M*NNZ i32
//                     LE words from the first slice of `input.bin`.
//   - `load_col_idx` — () -> Vec<i32>, effectful. Reads M*NNZ i32
//                     LE words from the second slice.
//   - `load_x`       — () -> Vec<i32>, effectful. Reads N i32 LE
//                     words from the third slice.
//   - `save_y`       — (Vec<i32>) -> (), effectful. Writes M i32
//                     LE words to the output path.
//
// Why `Vec<i32>` and not nested fixed arrays
// ------------------------------------------
// TASK-0103 (Done cycle 17) convention. Aggregate types in Nuc
// (`i32[M][NNZ]`, `i32[N]`, `i32[M]`) all spell to a flat `Vec<i32>`
// on the Rust side; the codegen flattens the multi-dim indexing.
// Same as every other example since cycle 17.
//
// Input file layout (`input.bin`, 224 bytes total)
// ------------------------------------------------
//   bytes [0   ..  96)  val[M][NNZ]    — M*NNZ i32 LE words.
//   bytes [96  .. 192)  col_idx[M][NNZ] — M*NNZ i32 LE words.
//   bytes [192 .. 224)  x[N]            — N i32 LE words.
//
// All three loaders read the same input.bin and slice their portion.
// Same multi-input pattern as 07-matmul (load_a + load_b).
//
// Output file layout (`output.bin`, 32 bytes)
// -------------------------------------------
//   bytes [0 .. 32) — y[M], M i32 LE words.
//
// Numeric type choice: i32 with wrapping_add / wrapping_mul
// ---------------------------------------------------------
// PRD §10.1 and docs/reference-impl-policy.md §5. Integer
// arithmetic is bit-deterministic across schedules and backends.
// The committed input pattern keeps every value small enough that
// overflow never trips in practice (see README); `wrapping_add` /
// `wrapping_mul` document the overflow contract.
//
// Contract pass (TASK-0012) expected behaviour against this file:
//   - PASS for `spmv_step` — declared `(i32,i32,i32,i32,i32) ->
//                            i32`, five scalar params, scalar
//                            return.
//   - `TypeMismatch` for `load_val`, `load_col_idx`, `load_x`,
//                            `save_y` — declared aggregate
//                            (`i32[M][NNZ]`, `i32[N]`, `i32[M]`),
//                            matched against `Vec<i32>` (scalar-
//                            only matcher at M1). Loud failure,
//                            pinned by the contract test. Same
//                            pattern as every other aggregate-IO
//                            example.
//
// I/O paths: NUC_INPUT_PATH / NUC_OUTPUT_PATH override the
// conventional sibling filenames in the cwd.

use std::env;
use std::fs;
use std::io::Write;

/// Matrix rows. Mirrors `const M : usize = 8;` in `prog.algo.nuc`.
/// The doubled declaration is the v2 convention per TASK-0103:
/// kernels.rs is plain Rust compiled unmodified by the host
/// toolchain; Nucleus does not text-substitute algorithm consts
/// into kernel bodies.
const M: usize = 8;

/// Matrix columns / dense input-vector length. Mirrors `const N :
/// usize = 8;` in `prog.algo.nuc`.
const N: usize = 8;

/// Fixed nonzeros per row. Mirrors `const NNZ : usize = 3;` in
/// `prog.algo.nuc`.
const NNZ: usize = 3;

const BYTES_PER_WORD: usize = 4;
const VAL_ELEMS: usize = M * NNZ;
const COL_IDX_ELEMS: usize = M * NNZ;
const X_ELEMS: usize = N;
const VAL_BYTES: usize = VAL_ELEMS * BYTES_PER_WORD;
const COL_IDX_BYTES: usize = COL_IDX_ELEMS * BYTES_PER_WORD;
const X_BYTES: usize = X_ELEMS * BYTES_PER_WORD;
const VAL_OFFSET: usize = 0;
const COL_IDX_OFFSET: usize = VAL_BYTES;
const X_OFFSET: usize = VAL_BYTES + COL_IDX_BYTES;
const INPUT_BYTES: usize = VAL_BYTES + COL_IDX_BYTES + X_BYTES;
const Y_ELEMS: usize = M;
const Y_BYTES: usize = Y_ELEMS * BYTES_PER_WORD;

/// Masked multiply-accumulate. Returns `acc + v*x_j` iff `j == c`,
/// else `acc`. The mask `j == c` is the data-dependent address
/// the algorithm sublanguage cannot spell at the IndexExpr level.
/// `wrapping_mul` and `wrapping_add` document the overflow
/// contract; the committed fixture's max per-row sum is far below
/// i32::MAX (see README).
pub fn spmv_step(acc: i32, v: i32, c: i32, x_j: i32, j: i32) -> i32 {
    if j == c {
        acc.wrapping_add(v.wrapping_mul(x_j))
    } else {
        acc
    }
}

/// Read the val matrix from the first slice of input.bin.
pub fn load_val() -> Vec<i32> {
    let bytes = read_input_bin();
    decode_slice(&bytes, VAL_OFFSET, VAL_ELEMS)
}

/// Read the col_idx matrix from the second slice of input.bin.
pub fn load_col_idx() -> Vec<i32> {
    let bytes = read_input_bin();
    decode_slice(&bytes, COL_IDX_OFFSET, COL_IDX_ELEMS)
}

/// Read the x vector from the third slice of input.bin.
pub fn load_x() -> Vec<i32> {
    let bytes = read_input_bin();
    decode_slice(&bytes, X_OFFSET, X_ELEMS)
}

fn read_input_bin() -> Vec<u8> {
    let path = env::var("NUC_INPUT_PATH").unwrap_or_else(|_| "input.bin".to_string());
    let bytes =
        fs::read(&path).unwrap_or_else(|e| panic!("load: cannot read {}: {}", path, e));
    assert!(
        bytes.len() >= INPUT_BYTES,
        "load: file {} has {} bytes; need at least {} (M*NNZ*4 + M*NNZ*4 + N*4 = {}+{}+{}; M={}, NNZ={}, N={})",
        path,
        bytes.len(),
        INPUT_BYTES,
        VAL_BYTES,
        COL_IDX_BYTES,
        X_BYTES,
        M,
        NNZ,
        N
    );
    bytes
}

fn decode_slice(bytes: &[u8], offset: usize, elems: usize) -> Vec<i32> {
    let mut out = Vec::with_capacity(elems);
    for k in 0..elems {
        let off = offset + k * BYTES_PER_WORD;
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

pub fn save_y(y: Vec<i32>) {
    assert_eq!(
        y.len(),
        Y_ELEMS,
        "save_y: expected {} elements (M = {}), got {}",
        Y_ELEMS,
        M,
        y.len()
    );
    let path = env::var("NUC_OUTPUT_PATH").unwrap_or_else(|_| "output.bin".to_string());
    let mut bytes = Vec::with_capacity(Y_BYTES);
    for v in &y {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    let mut f = fs::File::create(&path)
        .unwrap_or_else(|e| panic!("save_y: cannot create {}: {}", path, e));
    f.write_all(&bytes)
        .unwrap_or_else(|e| panic!("save_y: write failed: {}", e));
}
