// Kernel bodies for example 28-bin-fsum (per-bin reproducible float sum).
//
// Mirrors 27-bin-fmin's I/O layout: input.bin packs a key stream (N
// i32 LE words) followed by a val stream (N f32 LE words). The only
// compute kernel, `bin_fsum`, is the per-(element, bin) fold step.
//
// Why `Vec<i32>` / `Vec<f32>` and not `[T; N]`: same TASK-0103
// convention as every other example — aggregate kernel signatures use
// runtime-length-checked Vecs.
//
// Determinism (PRD §10.1): the cross-backend bit-identity of the float
// sum comes from the FIXED fold order the compiler emits (per-worker
// slice fold + worker-id-sorted host fan-in), NOT from this kernel —
// `bin_fsum` is a single `+`. The values are strictly positive finite
// f32 (see reference/ README); the host fan-in pre-inits each bucket to
// 0.0 (the fsum identity).

use std::env;
use std::fs;
use std::io::Write;

const N: usize = 256;
const BINS: usize = 16;

/// Per-(element, bin) fold: add `v` into `acc` iff this element's key
/// selects this bin. A plain `+` — the reproducibility is the compiler's
/// FIXED fold order, not anything this kernel does.
pub fn bin_fsum(acc: f32, k: i32, v: f32, bin: i32) -> f32 {
    if k == bin {
        acc + v
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
