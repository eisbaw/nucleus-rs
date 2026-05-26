// Kernel bodies for example 08-histogram.
//
// Three kernels:
//   - `bin_inc(acc, value, bin)` — scalar (i32, i32, i32) -> i32,
//                                  pure. Masked increment: returns
//                                  `acc + 1` iff `value == bin`,
//                                  else `acc`. The data-dependent
//                                  indexing the algorithm language
//                                  cannot express (PRD §6.2.4: no
//                                  conditionals) lives here.
//   - `load_input()`             — () -> Vec<i32>, effectful. Reads
//                                  N i32 LE words from `input.bin`
//                                  (or `$NUC_INPUT_PATH`).
//   - `save_output(h)`           — (Vec<i32>) -> (), effectful.
//                                  Writes the BINS-wide histogram
//                                  as BINS * 4 LE bytes to
//                                  `output.bin` (or
//                                  `$NUC_OUTPUT_PATH`).
//
// Why `Vec<i32>` and not `[i32; N]` / `[i32; BINS]`
// --------------------------------------------------
// Same reasoning as examples 01 / 02 / 03 / 04: PRD const-in-Rust-
// generics flow is unresolved (TASK-0103). `Vec<i32>` carries
// length at runtime; we check it explicitly in `load_input` and
// `save_output` so shape drift surfaces as a runtime panic rather
// than silent acceptance. Resolves when TASK-0103 picks a
// convention.
//
// Contract pass (TASK-0012) expected behaviour at cycle 186:
//   - PASS for `bin_inc`         — declared `(i32, i32, i32) -> i32`,
//                                  matches.
//   - `TypeMismatch` for
//     `load_input` AND
//     `save_output`              — both have aggregate types
//                                  (`i32[N]`, `i32[BINS]`); the
//                                  scalar-only matcher emits the
//                                  "aggregate type matching not
//                                  yet implemented" diagnostic.
//                                  Loud failure, not silent
//                                  acceptance. Same shape as
//                                  examples 01 / 02 / 03 / 04
//                                  pin.
// When aggregate matching lands (TASK-0103 picks the convention),
// these examples need no change; the matcher learns to accept
// `Vec<i32>` as `i32[N]`.
//
// I/O paths: same convention as examples 01 / 02 / 03 / 04. Read
// from `NUC_INPUT_PATH` / `NUC_OUTPUT_PATH` if set, else
// conventional sibling filenames in the cwd.

use std::env;
use std::fs;
use std::io::Write;

/// Input length used by the algorithm. Mirrors `const N : usize =
/// 256;` in `prog.algo.nuc`. Single-source-of-truth violation
/// (TASK-0103); disappears when the const-flow convention is
/// picked.
const N: usize = 256;

/// Histogram bin count. Mirrors `const BINS : usize = 16;` in
/// `prog.algo.nuc`. Same TASK-0103 caveat as `N`.
const BINS: usize = 16;

pub fn bin_inc(acc: i32, value: i32, bin: i32) -> i32 {
    // Masked increment. Returns `acc + 1` iff `value == bin`, else
    // `acc`. `wrapping_add` documents the overflow contract
    // explicitly (PRD §10.1: integer arithmetic is bit-
    // deterministic). The committed fixture's max bin count is
    // far below i32::MAX, but the choice is defensive — a
    // pathological future fixture that overflows wraps cleanly
    // rather than panicking on debug or invoking UB on release.
    if value == bin {
        acc.wrapping_add(1)
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

pub fn save_output(h: Vec<i32>) {
    assert_eq!(
        h.len(),
        BINS,
        "save_output: histogram has {} elements; expected {} (BINS)",
        h.len(),
        BINS
    );
    let path = env::var("NUC_OUTPUT_PATH").unwrap_or_else(|_| "output.bin".to_string());
    let mut buf = Vec::with_capacity(BINS * 4);
    for v in &h {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    let mut f = fs::File::create(&path)
        .unwrap_or_else(|e| panic!("save_output: cannot create {}: {}", path, e));
    f.write_all(&buf)
        .unwrap_or_else(|e| panic!("save_output: write failed: {}", e));
}
