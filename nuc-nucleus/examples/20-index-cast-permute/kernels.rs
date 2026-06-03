// Kernel bodies for 20-index-cast-permute (TASK-0431 AC#2 — a PURE index
// kernel called with a BARE ITER VAR argument).
//
// Paired with `prog.algo.nuc` by the e2e harness's default variant rule
// (TASK-0049.08): `prog.algo.nuc` <-> `kernels.rs`. Plain Rust compiled
// by the host toolchain unmodified — Nucleus does not text-substitute
// algorithm consts, so `N` is duplicated here per the TASK-0103
// convention.

use std::env;
use std::fs;
use std::io::Write;

/// Array length. Mirrors `const N : usize = 256;` in `prog.algo.nuc`.
const N: usize = 256;

/// Index-computing kernel — PURE, called in INDEX position with a BARE
/// ITER VAR argument (`out[idx(i)] <-- ...`). `idx(i) = N-1-i` (the
/// REVERSAL bijection over `0..N`).
///
/// The reversal (rather than the identity) is deliberate (TASK-0431.01):
/// it makes the oracle VALUE-DISCRIMINATING — `out` is `src` reversed,
/// so a backend that merely copied `src`→`out` WITHOUT evaluating `idx`
/// would now mismatch `reference.bin`, whereas an identity `idx` left the
/// oracle satisfiable by a plain copy. It is still a bijection over
/// `0..N`, so each `out` slot is written exactly once (PRD §6.2.1).
///
/// The `i32` parameter type is the whole point of this example: the loop
/// iter var renders `i64` in the generated host source, so the codegen
/// MUST cast `(i) as i32` at the call site (TASK-0431) or the generated
/// crate fails E0308. This kernel body is deterministic and
/// side-effect-free, so it is sound in index position (PRD §6.2).
pub fn idx(i: i32) -> i32 {
    (N as i32) - 1 - i
}

/// Identity passthrough for the value (15-transpose `xpose` precedent).
/// `pass(x) = x`. Pure.
pub fn pass(x: i32) -> i32 {
    x
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

pub fn save_output(o: Vec<i32>) {
    assert_eq!(
        o.len(),
        N,
        "save_output: out has {} elements; expected {} (N)",
        o.len(),
        N
    );
    let path = env::var("NUC_OUTPUT_PATH").unwrap_or_else(|_| "output.bin".to_string());
    let mut buf = Vec::with_capacity(N * 4);
    for v in &o {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    let mut f = fs::File::create(&path)
        .unwrap_or_else(|e| panic!("save_output: cannot create {}: {}", path, e));
    f.write_all(&buf)
        .unwrap_or_else(|e| panic!("save_output: write failed: {}", e));
}
