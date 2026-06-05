// Kernel bodies for example 22-dma-pio-demo.
//
// Structurally a clone of example 02-split-add's `kernels.rs` (same
// per-kernel Rust shape) — duplicated here so the example is
// self-contained per PRD §3 ("kernel bodies live in adjacent Rust
// source"). Sharing code with example 02 via a path-dependency would
// violate the example-locality principle and complicate
// reference-impl independence audits.
//
// Four kernels:
//   - `apply_gain`   — scalar (i32, i32) -> i32, PURE. Q8 fixed-point
//                      gain: `(sample * gain) >> 8`, so `gain = 256`
//                      is unity. This is the ONLY body the embedded
//                      (no_std) backend extracts verbatim, so it MUST
//                      be no_std-clean: wrapping integer ops only, no
//                      std, no float (it is).
//   - `load_samples` — () -> Vec<i32>, effectful. Reads positions
//                      0..N from `input.bin` (the bulk frame).
//   - `load_gains`   — () -> Vec<i32>, effectful. Reads positions
//                      N..2N from `input.bin` (the gain coefficients).
//   - `save_output`  — (Vec<i32>) -> (), effectful.
//
// Embedded-backend note (TASK-0438.03 / TASK-0049 mechanism): the
// embedded-pattern backend does NOT extract the effectful `load_*` /
// `save_*` bodies. On the emulated MCU `load_*` auto-fills its array
// from the Renode-injected input region IN FIRE ORDER, and
// `save_output` drains its array to USART1. So the std file-I/O
// bodies below are for tier-1 (pthreads/host) completeness and for
// running the reference-style flow on the host; the embedded emit
// ignores them. ONLY `apply_gain` (pure) is emitted verbatim, which
// is why it alone must be no_std-clean.
//
// Why `Vec<i32>` and not `[i32; N]`: same rationale as example 02 —
// see that example's README and TASK-0103. `Vec<i32>` carries length
// at runtime; we check it explicitly in `save_output`.
//
// I/O paths: read from environment variables when set
// (`NUC_INPUT_PATH` / `NUC_OUTPUT_PATH`, threaded by the host
// backend), fall back to conventional sibling filenames.

use std::env;
use std::fs;
use std::io::Write;

/// Length used by the algorithm. Mirrors `const N : usize = 256;` in
/// `prog.algo.nuc`. The duplication is the same single-source-of-truth
/// caveat as example 02 (TASK-0103); it disappears when codegen passes
/// the Nuc const into kernels.rs.
const N: usize = 256;

/// Q8 fixed-point gain-apply. PURE and no_std-clean: a single
/// `wrapping_mul` (deterministic two's-complement overflow per
/// PRD §10.1) followed by an arithmetic right shift of 8 (divide by
/// 256). `gain = 256` is unity. This body is extracted verbatim into
/// the no_std embedded firmware, so it must contain no std and no
/// float — it does not.
pub fn apply_gain(sample: i32, gain: i32) -> i32 {
    (sample.wrapping_mul(gain)) >> 8
}

pub fn load_samples() -> Vec<i32> {
    let path = env::var("NUC_INPUT_PATH").unwrap_or_else(|_| "input.bin".to_string());
    read_i32_le_slice(&path, 0, N)
}

pub fn load_gains() -> Vec<i32> {
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
    let bytes =
        fs::read(path).unwrap_or_else(|e| panic!("load: cannot read {}: {}", path, e));
    let need = (start + count) * 4;
    assert!(
        bytes.len() >= need,
        "load: file {} has {} bytes; need at least {}",
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
