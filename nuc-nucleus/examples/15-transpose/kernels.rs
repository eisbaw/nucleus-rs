// Kernel bodies for example 15-transpose. Landed cycle 204
// (TASK-0341.01 AC#1 language-sanity slice).
//
// PRD §6.2.2: kernel bodies live in an adjacent Rust source file and
// are real Rust functions. Nucleus does not interpolate text into
// these bodies; they are compiled by the host toolchain unmodified.
//
// Three kernels:
//   - `xpose(x)`     — scalar (i32) -> i32, pure identity passthrough.
//   - `load_input()` — () -> Vec<i32>, effectful. Reads H*W i32 LE words
//                      from `input.bin` (or `$NUC_INPUT_PATH`).
//   - `save_output(v)`— (Vec<i32>) -> (), effectful. Writes W*H i32 LE
//                       words to `output.bin` (or `$NUC_OUTPUT_PATH`).
//
// Why an identity kernel and not bare `out[j][i] <-- in[i][j]`
// -----------------------------------------------------------
// The algorithm grammar (docs/grammar-algo.md §1) allows a bare
// `LValue` on the RHS as an identity copy: `RValue ::= CallExpr |
// LValue`. But `acfg::build::build_dataflow` skips non-Call RHS at
// M1 (`nucleus/nucleus-compiler/src/acfg/build.rs:325-327`, "Identity
// copy or pure-expression RHS: skipped at M1"). With the bare-LValue
// form the ACFG would carry no Operation node for the transpose body,
// and the codegen would emit nothing into the loop. A pure kernel
// returning its argument is the canonical way to express "permute the
// indices and write the same value", and it lets the compiler see the
// per-element dataflow it needs to lay out under any future schedule.
//
// Why `Vec<i32>` and not `[i32; H*W]` / `[i32; W*H]`
// --------------------------------------------------
// TASK-0103 (Done cycle 17): `Vec<i32>` + runtime length check is the
// canonical convention for aggregate-typed kernel signatures. The
// algorithm declares `input : i32[H][W]` and `output : i32[W][H]`.
// On the Rust side these are flat `Vec<i32>` of length H*W and W*H
// respectively (numerically equal here since W*H = H*W = 128, but the
// shapes are conceptually different). The codegen flattens 2D index
// `input[i][j]` to `input[i * W + j]` and `output[j][i]` to
// `output[j * H + i]` using the data shape declarations.
//
// Contract pass (TASK-0012) expected behaviour against this file:
//   - PASS for `xpose`        — declared `(i32) -> i32`, scalar match.
//   - `TypeMismatch` for `load_input`, `save_output` — aggregate
//                              declarations matched against `Vec<i32>`
//                              (scalar-only matcher at M1). Loud
//                              failure pinned by the contract test.
//
// I/O paths: NUC_INPUT_PATH / NUC_OUTPUT_PATH override the conventional
// sibling filenames in the cwd. Matches examples 01/02/03/05.

use std::env;
use std::fs;
use std::io::Write;

/// Dimensions used by the algorithm. Mirror `const H : usize = 8;`
/// and `const W : usize = 16;` in `prog.algo.nuc`. The doubled
/// declaration is the v2 convention per TASK-0103 — kernels.rs is
/// plain Rust compiled unmodified; Nucleus does not text-substitute
/// algorithm consts into kernel bodies.
const H: usize = 8;
const W: usize = 16;
const N_IN: usize = H * W;
const N_OUT: usize = W * H;

/// Identity passthrough. The point of this example is the axis-swap
/// on the dataflow side, not the per-element computation; the kernel
/// body is therefore the simplest possible function.
pub fn xpose(x: i32) -> i32 {
    x
}

pub fn load_input() -> Vec<i32> {
    let path = env::var("NUC_INPUT_PATH").unwrap_or_else(|_| "input.bin".to_string());
    let bytes =
        fs::read(&path).unwrap_or_else(|e| panic!("load_input: cannot read {}: {}", path, e));
    let need = N_IN * 4;
    assert!(
        bytes.len() >= need,
        "load_input: file {} has {} bytes; need at least {} (H*W = {}*{} = {})",
        path,
        bytes.len(),
        need,
        H,
        W,
        N_IN
    );
    let mut out = Vec::with_capacity(N_IN);
    for i in 0..N_IN {
        let off = i * 4;
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

pub fn save_output(data: Vec<i32>) {
    assert_eq!(
        data.len(),
        N_OUT,
        "save_output: expected {} elements (W*H = {}*{}), got {}",
        N_OUT,
        W,
        H,
        data.len()
    );
    let path = env::var("NUC_OUTPUT_PATH").unwrap_or_else(|_| "output.bin".to_string());
    let mut bytes = Vec::with_capacity(data.len() * 4);
    for v in &data {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    let mut f = fs::File::create(&path)
        .unwrap_or_else(|e| panic!("save_output: cannot create {}: {}", path, e));
    f.write_all(&bytes)
        .unwrap_or_else(|e| panic!("save_output: write failed: {}", e));
}
