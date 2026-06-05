// Kernel bodies for example 27-bin-fmin (TASK-0343.02).
//
// The FLOAT (f32) analog of 26-bin-min/kernels.rs. Four kernels:
//   - `bin_fmin(acc, k, v, bin)` — scalar (f32, i32, f32, i32) -> f32,
//                                  pure. Masked min-fold: returns
//                                  `acc.min(v)` iff `k == bin`, else
//                                  `acc`. The data-dependent indexing
//                                  the algorithm language cannot express
//                                  (PRD §6.2.4: no conditionals) lives
//                                  here.
//   - `load_key()`               — () -> Vec<i32>, effectful. Reads N
//                                  i32 LE words from input.bin positions
//                                  [0..N) (each in [0, BINS)).
//   - `load_val()`               — () -> Vec<f32>, effectful. Reads N
//                                  f32 LE words from input.bin positions
//                                  [N..2N) (each strictly positive,
//                                  finite, NaN-free).
//   - `save_output(r)`           — (Vec<f32>) -> (), effectful. Writes
//                                  the BINS-wide result as BINS * 4 LE
//                                  bytes (f32 little-endian) to
//                                  output.bin.
//
// Why `Vec<f32>` and not `[f32; N]` / `[f32; BINS]`: TASK-0103 — the
// canonical aggregate-typed kernel-signature convention (runtime length
// check; shape drift surfaces as a panic, not silent acceptance).
//
// Float byte convention: f32 LITTLE-ENDIAN, 4 bytes per word — the same
// `to_le_bytes` / `from_le_bytes` convention examples 02-split-add /
// 13-cnn-inference use. An EMPTY bin's output is `f32::INFINITY`, whose
// LE bytes are 0x00 0x00 0x80 0x7F (bit pattern 0x7F800000).
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

pub fn bin_fmin(acc: f32, k: i32, v: f32, bin: i32) -> f32 {
    // Masked min-fold. Returns `acc.min(v)` iff `k == bin`, else `acc`.
    // Across the full rectangular (i, b) nest the per-bin result is the
    // minimum `v` over all inputs whose `k == bin` — or the identity
    // `f32::INFINITY` (the codegen pre-init) for a bin no input maps to.
    // `f32::min` is order-independent for the distinct finite positive
    // values this fixture commits to (PRD §10.1 bit-identity).
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

pub fn load_val() -> Vec<f32> {
    let path = env::var("NUC_INPUT_PATH").unwrap_or_else(|_| "input.bin".to_string());
    read_f32_le_slice(&path, N, N)
}

pub fn save_output(r: Vec<f32>) {
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
    let bytes = read_words(path, start, count);
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let off = (start + i) * 4;
        out.push(i32::from_le_bytes([
            bytes[off],
            bytes[off + 1],
            bytes[off + 2],
            bytes[off + 3],
        ]));
    }
    out
}

/// Read `count` little-endian f32 words from `path`, starting at
/// element offset `start`.
fn read_f32_le_slice(path: &str, start: usize, count: usize) -> Vec<f32> {
    let bytes = read_words(path, start, count);
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let off = (start + i) * 4;
        out.push(f32::from_le_bytes([
            bytes[off],
            bytes[off + 1],
            bytes[off + 2],
            bytes[off + 3],
        ]));
    }
    out
}

/// Read the whole file and assert it covers `(start + count)` 4-byte
/// words. Shared by the i32 / f32 readers so the length check cannot
/// drift between them.
fn read_words(path: &str, start: usize, count: usize) -> Vec<u8> {
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
    bytes
}
