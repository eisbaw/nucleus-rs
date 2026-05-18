// Kernel bodies for example 02-split-add.
//
// Identical in shape to example 01's `kernels.rs` (same per-kernel
// Rust semantics) — duplicated here so the example is self-contained
// per PRD §3 ("Exactly two source files per build: one algorithm, one
// schedule; kernel bodies live in adjacent Rust source"). Sharing
// code with example 01 via a path-dependency would violate the
// example-locality principle and complicate reference-impl
// independence audits (docs/reference-impl-policy.md §2 only governs
// the reference; kernels.rs is the algorithm-side artefact, but the
// same "no surprising coupling" preference applies).
//
// Three kernels:
//   - `add`          — scalar (i32, i32) -> i32, pure.
//   - `load_input`   — () -> Vec<i32>, effectful. Reads positions
//                      0..N from `input.bin`.
//   - `load_input_b` — () -> Vec<i32>, effectful. Reads positions
//                      N..2N from `input.bin`.
//   - `save_output`  — (Vec<i32>) -> (), effectful.
//
// Why `Vec<i32>` and not `[i32; N]`
// ---------------------------------
// See example 01's README and TASK-0103. Short version: `[i32; N]`
// would require Nuc-side const `N` to be a Rust const in the same
// file, which the PRD §6.2.2 example sketch does not specify yet.
// `Vec<i32>` carries length at runtime; we check it explicitly in
// `save_output`. Trade-off: shape error becomes a runtime panic
// rather than a compile-time mismatch. Tracked by TASK-0103.
//
// Contract pass (TASK-0012) behaviour expected against this file:
//   - PASS for `add` — declared `(i32, i32) -> i32`, signature
//     matches.
//   - `TypeMismatch` for `load_input`, `load_input_b`, `save_output`
//     because they are aggregate-typed in Nuc (`i32[N]`) and
//     scalar-only matching is what TASK-0012 ships at this stage.
//     Loud failure, not silent acceptance — pinned by the contract
//     test in `contract.rs`.
//
// I/O paths
// ---------
// Same convention as example 01: read paths from environment
// variables when set, fall back to conventional sibling filenames.
// This keeps the kernels testable in isolation and matches what the
// pthreads-sync-emitted host program threads in via `NUC_INPUT_PATH`
// / `NUC_OUTPUT_PATH`.

use std::env;
use std::fs;
use std::io::Write;

/// Length used by the algorithm. Mirrors `const N : usize = 256;`
/// in `prog.algo.nuc`. The duplication is the single-source-of-truth
/// violation called out in the header (and in TASK-0103). When the
/// Nucleus codegen passes consts into kernels.rs (or picks a Rust-
/// side surface), this duplicate disappears.
const N: usize = 256;

pub fn add(a: i32, b: i32) -> i32 {
    // Wrapping arithmetic: deterministic on overflow (two's complement
    // wraparound). PRD §10.1 tier-1 demands bit-identical output
    // across runs; default `+` panics in debug and is undefined for
    // the determinism contract under release.
    a.wrapping_add(b)
}

pub fn load_input() -> Vec<i32> {
    let path = env::var("NUC_INPUT_PATH").unwrap_or_else(|_| "input.bin".to_string());
    read_i32_le_slice(&path, 0, N)
}

pub fn load_input_b() -> Vec<i32> {
    let path = env::var("NUC_INPUT_PATH").unwrap_or_else(|_| "input.bin".to_string());
    read_i32_le_slice(&path, N, N)
}

pub fn save_output(data: Vec<i32>) {
    assert_eq!(
        data.len(),
        N,
        "save_output: expected {} elements, got {}",
        N,
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

/// Read `count` little-endian i32 words from `path`, starting at
/// element offset `start` (i.e. byte offset `start * 4`).
fn read_i32_le_slice(path: &str, start: usize, count: usize) -> Vec<i32> {
    let bytes = fs::read(path)
        .unwrap_or_else(|e| panic!("load_input: cannot read {}: {}", path, e));
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
