// Kernel bodies for the getting-started tutorial (scale-and-bias).
//
// PRD §6.2.2: kernel bodies are real Rust functions in an adjacent
// source file. Nucleus does NOT interpolate text into these bodies;
// the host toolchain compiles them unmodified.
//
// Four kernels:
//   - scale_bias — pure scalar `(i32, i32) -> i32`. Computes K*a + b.
//   - load_a     — effectful `() -> Vec<i32>`. Reads N i32 LE words
//                  from input.bin at element positions 0..N.
//   - load_b     — effectful `() -> Vec<i32>`. Reads N i32 LE words
//                  from input.bin at element positions N..2N.
//   - save_c     — effectful `(Vec<i32>) -> ()`. Writes N i32 LE words
//                  to NUC_OUTPUT_PATH (or output.bin).
//
// I/O paths come from env vars (NUC_INPUT_PATH / NUC_OUTPUT_PATH) so
// the generated program is location-independent — this is how the
// runtime wires inputs/outputs, and how `just tutorial` points two
// backends at the same input and diffs their outputs.

use std::env;
use std::fs;
use std::io::Write;

/// Array length. Mirrors `const N : usize = 64;` in prog.algo.nuc.
/// The duplication is the documented v2 trade-off (the Rust signatures
/// use `Vec<i32>`, which carries length at runtime, not in the type).
const N: usize = 64;

/// Integer scale factor. The `K` referenced in the algorithm's
/// `c[i] = K * a[i] + b[i]` comment lives here, in the kernel body.
const K: i32 = 3;

/// Pure scalar kernel: `K * a + b`.
///
/// Wrapping arithmetic is deliberate: it is deterministic on overflow
/// (two's-complement wraparound), unlike `*`/`+` which panic in debug
/// and break the bit-identical determinism contract (PRD §10.1).
pub fn scale_bias(a: i32, b: i32) -> i32 {
    K.wrapping_mul(a).wrapping_add(b)
}

pub fn load_a() -> Vec<i32> {
    let path = env::var("NUC_INPUT_PATH").unwrap_or_else(|_| "input.bin".to_string());
    read_i32_le_slice(&path, 0, N)
}

pub fn load_b() -> Vec<i32> {
    let path = env::var("NUC_INPUT_PATH").unwrap_or_else(|_| "input.bin".to_string());
    read_i32_le_slice(&path, N, N)
}

pub fn save_c(data: Vec<i32>) {
    assert_eq!(
        data.len(),
        N,
        "save_c: expected {} elements, got {}",
        N,
        data.len()
    );
    let path = env::var("NUC_OUTPUT_PATH").unwrap_or_else(|_| "output.bin".to_string());
    let mut bytes = Vec::with_capacity(data.len() * 4);
    for v in &data {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    let mut f = fs::File::create(&path)
        .unwrap_or_else(|e| panic!("save_c: cannot create {}: {}", path, e));
    f.write_all(&bytes)
        .unwrap_or_else(|e| panic!("save_c: write failed: {}", e));
}

/// Read `count` little-endian i32 words from `path`, starting at
/// element offset `start` (byte offset `start * 4`).
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
