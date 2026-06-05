// Kernel bodies for example 25-bin-parity (TASK-0343.01.01).
//
// The XOR sibling of 08-histogram/kernels.rs. Three kernels:
//   - `bin_xor(acc, value, bin)` — scalar (i32, i32, i32) -> i32,
//                                  pure. Masked parity toggle: returns
//                                  `acc ^ 1` iff `value == bin`, else
//                                  `acc`. The data-dependent indexing
//                                  the algorithm language cannot express
//                                  (PRD §6.2.4: no conditionals) lives
//                                  here.
//   - `load_input()`             — () -> Vec<i32>, effectful. Reads N
//                                  i32 LE words from `input.bin` (or
//                                  `$NUC_INPUT_PATH`). Identical to
//                                  08-histogram (same `input.bin`).
//   - `save_output(p)`           — (Vec<i32>) -> (), effectful. Writes
//                                  the BINS-wide parity as BINS * 4 LE
//                                  bytes to `output.bin` (or
//                                  `$NUC_OUTPUT_PATH`).
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

/// Parity bin count. Mirrors `const BINS : usize = 16;`. Same TASK-0103
/// convention as `N`.
const BINS: usize = 16;

pub fn bin_xor(acc: i32, value: i32, bin: i32) -> i32 {
    // Masked parity toggle. Returns `acc ^ 1` iff `value == bin`, else
    // `acc`. Across the full rectangular (i, b) nest the per-bin result
    // is `(count of inputs equal to bin) mod 2` — the bin's parity.
    if value == bin {
        acc ^ 1
    } else {
        acc
    }
}

pub fn load_input() -> Vec<i32> {
    let path = env::var("NUC_INPUT_PATH").unwrap_or_else(|_| "input.bin".to_string());
    let bytes =
        fs::read(&path).unwrap_or_else(|e| panic!("load_input: cannot read {}: {}", path, e));
    let need = N * 4;
    assert!(
        bytes.len() >= need,
        "load_input: file {} has {} bytes; need at least {} (N={})",
        path,
        bytes.len(),
        need,
        N
    );
    let mut out = Vec::with_capacity(N);
    for i in 0..N {
        let off = i * 4;
        let word =
            i32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
        out.push(word);
    }
    out
}

pub fn save_output(p: Vec<i32>) {
    assert_eq!(
        p.len(),
        BINS,
        "save_output: parity has {} elements; expected {} (BINS)",
        p.len(),
        BINS
    );
    let path = env::var("NUC_OUTPUT_PATH").unwrap_or_else(|_| "output.bin".to_string());
    let mut buf = Vec::with_capacity(BINS * 4);
    for v in &p {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    let mut f = fs::File::create(&path)
        .unwrap_or_else(|e| panic!("save_output: cannot create {}: {}", path, e));
    f.write_all(&buf)
        .unwrap_or_else(|e| panic!("save_output: write failed: {}", e));
}
