// Kernel bodies for example 01-elementwise-add.
//
// PRD §6.2.2: kernel bodies live in an adjacent Rust source file and
// are real Rust functions. Nucleus does not interpolate text into
// these bodies; they are compiled by the host toolchain unmodified.
//
// Three kernels:
//   - `add`         — scalar (i32, i32) -> i32, pure.
//   - `load_input`  — () -> Vec<i32>, effectful. Reads N i32 LE words
//                     from `input.bin` (positions 0..N).
//   - `load_input_b`— () -> Vec<i32>, effectful. Reads N i32 LE words
//                     from `input.bin` (positions N..2N).
//   - `save_output` — (Vec<i32>) -> (), effectful. Writes N i32 LE
//                     words to `reference.bin` (or whichever path the
//                     runtime sets via NUC_OUTPUT_PATH).
//
// Why `Vec<i32>` and not `[i32; N]` in the I/O kernel signatures
// -------------------------------------------------------------
// The PRD §6.2.2 example uses `Box<[[f32; W]; H]>` where W and H are
// Nuc-side const declarations. That signature DOES NOT compile as
// plain Rust because W and H are not Rust constants. See TASK-0103
// for the open PRD bug.
//
// This example sidesteps the issue by using `Vec<i32>` for array
// arguments: a heap-allocated, length-carrying owned buffer. Length
// is checked at runtime against the algorithm's `const N`. Trade-off:
// the Rust signature loses static shape — a wrong-length input
// becomes a runtime error rather than a compile error. Acceptable
// for v2 (TASK-0103's resolution will tighten this).
//
// What the contract check sees
// ----------------------------
// The contract pass (`check_kernels_contract`, TASK-0012) is scalar-
// only at present. It will:
//   - PASS for `add` (declared `(i32, i32) -> i32`, actual matches).
//   - REPORT `TypeMismatch` with "aggregate type matching is not yet
//     implemented" for `load_input`, `load_input_b`, `save_output`
//     because their declared types `i32[N]` are aggregates. This is
//     a known TASK-0012 limitation (loud failure rather than silent
//     acceptance); it is not a bug in this example. When aggregate
//     matching lands, the `Vec<i32>` signature here will need to be
//     accepted by the matcher (likely as one of the recognised
//     aggregate spellings).
//
// I/O paths
// ---------
// Effectful kernels read paths from environment variables when set,
// falling back to the conventional sibling files. This keeps the
// kernels independently testable (set the env vars to point at
// fixtures) and matches what a future runtime is likely to do.

use std::env;
use std::fs;
use std::io::Write;

/// Length used by the algorithm. Mirrors `const N : usize = 256;`
/// in `prog.algo.nuc`. Duplicating this here is the trade-off called
/// out in the file header (and in TASK-0103). When the Nucleus
/// codegen passes consts through to Rust, this duplicate goes away.
const N: usize = 256;

pub fn add(a: i32, b: i32) -> i32 {
    // Wrapping arithmetic: deterministic on overflow (two's complement
    // wraparound), unlike default `+` which panics in debug and is
    // undefined-equivalent for the determinism contract. PRD §10.1
    // tier-1 demands bit-identical output across runs.
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
