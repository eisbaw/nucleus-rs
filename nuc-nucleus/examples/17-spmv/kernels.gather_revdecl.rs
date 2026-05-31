// Kernel bodies for the 17-spmv REVERSED-DECLARATION native-gather
// FIFO-robustness fixture (`prog.gather_revdecl.algo.nuc`, TASK-0389).
//
// This is a VERBATIM functional copy of `kernels.gather.rs` (same
// loaders, same `gather_madd`, same constants, same input.bin layout).
// It exists ONLY because the e2e variant rule (TASK-0049.08) derives the
// kernels filename from the program stem: `prog.gather_revdecl.algo.nuc`
// <-> `kernels.gather_revdecl.rs`. The kernels are byte-identical to
// `kernels.gather.rs` because the reversed-declaration variant differs
// ONLY in the .nuc declaration/load ORDER, not in any kernel body.
//
// KEEP IN SYNC with `kernels.gather.rs`: if a loader or `gather_madd`
// changes there, mirror it here (they MUST stay byte-identical, since
// both drive the same input.bin/reference.bin oracle). A near-zero-cost
// drift guard lives in the contract pass's standalone compile of each
// kernels file; a value drift would surface as an e2e output.bin diff.
//
// The loaders are OFFSET-KEYED (`load_val` reads VAL_OFFSET, `load_x`
// reads X_OFFSET, `load_col_idx` reads COL_IDX_OFFSET), so they are
// order-INDEPENDENT: reversing the .nuc load order does not change which
// bytes each loader reads. Same input.bin, same reference.bin, same
// M/N/NNZ, same numerics as `prog.gather.algo.nuc`.

use std::env;
use std::fs;
use std::io::Write;

const M: usize = 8;
const N: usize = 8;
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

/// Native-gather multiply-accumulate: `acc + v * x_val`.
///
/// No mask and no `j`-fold — the data-dependent read
/// `x[col_idx[i][k]]` is expressed directly in `prog.gather.algo.nuc`,
/// so the gathered `x_val` is exactly the contributing element. Contrast
/// `kernels.rs::spmv_step`, whose mask + dense `j`-scan emulated the
/// gather the grammar once could not lower. `wrapping_*` documents the
/// overflow contract (PRD §10.1); the committed fixture stays far below
/// `i32::MAX`.
pub fn gather_madd(acc: i32, v: i32, x_val: i32) -> i32 {
    acc.wrapping_add(v.wrapping_mul(x_val))
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
    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("load: cannot read {}: {}", path, e));
    assert!(
        bytes.len() >= INPUT_BYTES,
        "load: file {} has {} bytes; need at least {} (M={}, NNZ={}, N={})",
        path,
        bytes.len(),
        INPUT_BYTES,
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
        let word = i32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
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
