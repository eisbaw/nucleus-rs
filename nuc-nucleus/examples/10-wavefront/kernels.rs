// Kernel bodies for example 10-wavefront.
//
// PRD §6.2.2: kernel bodies are real Rust functions in an adjacent
// file, compiled by the host toolchain unmodified. Nucleus does NOT
// interpolate text into them.
//
// Three kernels:
//   - `wavefront(in_cost)` — pure. Returns the H*W accumulated-cost
//                            matrix per the recurrence in
//                            prog.algo.nuc's header.
//   - `load_input()`       — () -> Vec<i32>, effectful. Reads H*W
//                            i32 LE words from `input.bin` (or
//                            `$NUC_INPUT_PATH`).
//   - `save_output(v)`     — (Vec<i32>) -> (), effectful. Writes
//                            H*W i32 LE words to `output.bin` (or
//                            `$NUC_OUTPUT_PATH`).
//
// Why the recurrence lives here and not in the algorithm
// -------------------------------------------------------
// See `prog.algo.nuc` header. Nuc v2 has no conditionals, no
// boundary-safe index form, and single-assignment is per data SYMBOL
// — so the natural `out[i][j] = min(out[i-1][j], ...)` recurrence is
// not expressible at the algorithm level. The honest workaround is
// to push the entire recurrence into a single pure kernel.
//
// Why `Vec<i32>` and not `[i32; H*W]`
// -----------------------------------
// Per TASK-0103 (Done cycle 17): `Vec<i32>` + runtime length check
// IS the canonical convention for aggregate-typed kernel signatures.
// The PRD §6.2.2 sketch `Box<[[f32; W]; H]>` did not compile as plain
// Rust (W and H are not Rust constants); `Vec<i32>` with explicit
// length checks is the resolution. Trade-off: shape mismatch is a
// runtime panic, not a compile error.
//
// I/O paths: same convention as the other examples — read
// `NUC_INPUT_PATH` / `NUC_OUTPUT_PATH` if set, else sibling filenames
// in cwd.

use std::env;
use std::fs;
use std::io::Write;

/// Wavefront dimensions. Mirror `const H : usize = 16; const W :
/// usize = 16;` in `prog.algo.nuc`. The doubled declaration is the
/// v2 convention per TASK-0103 (Done cycle 17): kernels.rs is plain
/// Rust compiled by the host toolchain unmodified — Nucleus does not
/// text-substitute algorithm consts into kernel bodies.
const H: usize = 16;
const W: usize = 16;
const N: usize = H * W;

/// The wavefront recurrence. Row-major sequential sweep — each cell
/// (i, j) reads cells (i-1, j), (i, j-1), (i-1, j-1) which were
/// written in earlier iterations of this same function.
///
/// `wrapping_add` documents overflow intent; the committed fixture
/// keeps the accumulated cost in range (see README and the reference
/// crate's `--gen-input` mode for the input range).
pub fn wavefront(in_cost: Vec<i32>) -> Vec<i32> {
    assert_eq!(
        in_cost.len(),
        N,
        "wavefront: in_cost has {} elements; need {} (H*W = {}*{})",
        in_cost.len(),
        N,
        H,
        W
    );
    let mut out = vec![0i32; N];
    // (0, 0) seed.
    out[0] = in_cost[0];
    // First row: each cell extends the previous (W direction only).
    for j in 1..W {
        out[j] = in_cost[j].wrapping_add(out[j - 1]);
    }
    // First column: each cell extends the previous (N direction only).
    for i in 1..H {
        out[i * W] = in_cost[i * W].wrapping_add(out[(i - 1) * W]);
    }
    // Interior cells: min of NW / N / W parents, then add input.
    for i in 1..H {
        for j in 1..W {
            let nw = out[(i - 1) * W + (j - 1)];
            let n = out[(i - 1) * W + j];
            let w = out[i * W + (j - 1)];
            let m = nw.min(n).min(w);
            out[i * W + j] = in_cost[i * W + j].wrapping_add(m);
        }
    }
    out
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
        let word = i32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
        out.push(word);
    }
    out
}

pub fn save_output(v: Vec<i32>) {
    assert_eq!(
        v.len(),
        N,
        "save_output: v has {} elements; need {} (H*W = {}*{})",
        v.len(),
        N,
        H,
        W
    );
    let path = env::var("NUC_OUTPUT_PATH").unwrap_or_else(|_| "output.bin".to_string());
    let mut f = fs::File::create(&path)
        .unwrap_or_else(|e| panic!("save_output: cannot create {}: {}", path, e));
    let mut bytes = Vec::with_capacity(N * 4);
    for w in v {
        bytes.extend_from_slice(&w.to_le_bytes());
    }
    f.write_all(&bytes)
        .unwrap_or_else(|e| panic!("save_output: write failed: {}", e));
}
