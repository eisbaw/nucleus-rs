// Kernel bodies for example 18-multigather.
//
// Self-contained per PRD §3 (one algorithm, one schedule, adjacent
// kernel bodies). Three kernels:
//   - `add`     — scalar (i32, i32) -> i32, pure. `wrapping_add` for
//                 deterministic overflow (PRD §10.1).
//   - `load_a`  — () -> Vec<i32>, effectful. Reads positions 0..N from
//                 `input.bin`.
//   - `save_pq` — (Vec<i32>, Vec<i32>) -> (), effectful. Writes `p` to
//                 the first N words of output.bin and `q` to the next N
//                 words (so a SINGLE output.bin / reference.bin oracle
//                 covers both loop-output arrays).
//
// I/O paths: read from NUC_INPUT_PATH / NUC_OUTPUT_PATH when set, else
// conventional sibling filenames — matches what the backends thread in.

use std::env;
use std::fs;
use std::io::Write;

/// Length used by the algorithm. Mirrors `const N : usize = 64;` in
/// `prog.algo.nuc`. Single-source-of-truth violation tracked the same
/// way as the other examples (TASK-0103).
const N: usize = 64;

pub fn add(a: i32, b: i32) -> i32 {
    a.wrapping_add(b)
}

pub fn load_a() -> Vec<i32> {
    let path = env::var("NUC_INPUT_PATH").unwrap_or_else(|_| "input.bin".to_string());
    read_i32_le_slice(&path, 0, N)
}

pub fn save_pq(p: Vec<i32>, q: Vec<i32>) {
    assert_eq!(p.len(), N, "save_pq: expected {} elements in p, got {}", N, p.len());
    assert_eq!(q.len(), N, "save_pq: expected {} elements in q, got {}", N, q.len());
    let path = env::var("NUC_OUTPUT_PATH").unwrap_or_else(|_| "output.bin".to_string());
    let mut bytes = Vec::with_capacity(2 * N * 4);
    for v in &p {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    for v in &q {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    let mut f = fs::File::create(&path)
        .unwrap_or_else(|e| panic!("save_pq: cannot create {}: {}", path, e));
    f.write_all(&bytes)
        .unwrap_or_else(|e| panic!("save_pq: write failed: {}", e));
}

/// Read `count` little-endian i32 words from `path`, starting at
/// element offset `start` (byte offset `start * 4`).
fn read_i32_le_slice(path: &str, start: usize, count: usize) -> Vec<i32> {
    let bytes =
        fs::read(path).unwrap_or_else(|e| panic!("load_a: cannot read {}: {}", path, e));
    let need = (start + count) * 4;
    assert!(
        bytes.len() >= need,
        "load_a: file {} has {} bytes; need at least {}",
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
