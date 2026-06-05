// Kernel bodies for example 26-bin-min (TASK-0343.01.02).
//
// The MIN sibling of 08-histogram/kernels.rs and the non-zero-identity
// sibling of 25-bin-parity/kernels.rs. Four kernels:
//   - `bin_min(acc, k, v, bin)` — scalar (i32, i32, i32, i32) -> i32,
//                                 pure. Masked min-fold: returns
//                                 `min(acc, v)` iff `k == bin`, else
//                                 `acc`. The data-dependent indexing the
//                                 algorithm language cannot express
//                                 (PRD §6.2.4: no conditionals) lives
//                                 here.
//   - `load_key()`              — () -> Vec<i32>, effectful. Reads N
//                                 i32 LE words from input.bin positions
//                                 [0..N) (each in [0, BINS)).
//   - `load_val()`              — () -> Vec<i32>, effectful. Reads N
//                                 i32 LE words from input.bin positions
//                                 [N..2N) (each strictly positive).
//   - `save_output(r)`          — (Vec<i32>) -> (), effectful. Writes
//                                 the BINS-wide result as BINS * 4 LE
//                                 bytes to output.bin.
//
// Why `Vec<i32>` and not `[i32; N]` / `[i32; BINS]`: TASK-0103 — the
// canonical aggregate-typed kernel-signature convention (runtime length
// check; shape drift surfaces as a panic, not silent acceptance).
//
// I/O paths: `NUC_INPUT_PATH` / `NUC_OUTPUT_PATH` if set, else the
// conventional sibling filenames in the cwd.

use std::env;
use std::fs;
use std::io::Write;

/// Input length used by the algorithm. Mirrors `const N : usize = 256;`
/// in `prog.algo.nuc` (the v2 doubled-decl convention per TASK-0103 —
/// kernels.rs is plain Rust compiled unmodified; Nucleus does not
/// text-substitute algorithm consts into kernel bodies).
const N: usize = 256;

/// Min-bin count. Mirrors `const BINS : usize = 16;`. Same TASK-0103
/// convention as `N`.
const BINS: usize = 16;

pub fn bin_min(acc: i32, k: i32, v: i32, bin: i32) -> i32 {
    // Masked min-fold. Returns `min(acc, v)` iff `k == bin`, else `acc`.
    // Across the full rectangular (i, b) nest the per-bin result is the
    // minimum `v` over all inputs whose `k == bin` — or the identity
    // `i32::MAX` (the codegen pre-init) for a bin no input maps to.
    if k == bin {
        acc.min(v)
    } else {
        acc
    }
}

pub fn load_key() -> Vec<i32> {
    let path = env::var("NUC_INPUT_PATH").unwrap_or_else(|_| "input.bin".to_string());
    read_i32_le_slice(&path, 0, N)
}

pub fn load_val() -> Vec<i32> {
    let path = env::var("NUC_INPUT_PATH").unwrap_or_else(|_| "input.bin".to_string());
    read_i32_le_slice(&path, N, N)
}

pub fn save_output(r: Vec<i32>) {
    assert_eq!(
        r.len(),
        BINS,
        "save_output: result has {} elements; expected {} (BINS)",
        r.len(),
        BINS
    );
    let path = env::var("NUC_OUTPUT_PATH").unwrap_or_else(|_| "output.bin".to_string());
    let mut buf = Vec::with_capacity(BINS * 4);
    for v in &r {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    let mut f = fs::File::create(&path)
        .unwrap_or_else(|e| panic!("save_output: cannot create {}: {}", path, e));
    f.write_all(&buf)
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
