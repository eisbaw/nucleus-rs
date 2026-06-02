// Kernel bodies for the 08-histogram TEXTBOOK-SCATTER variant
// (`prog.textbook.algo.nuc`, TASK-0430 — pure-kernel-call in index
// position).
//
// Paired with `prog.textbook.algo.nuc` by the e2e harness's variant rule
// (TASK-0049.08): `prog.textbook.algo.nuc` <-> `kernels.textbook.rs`. It
// is SELF-CONTAINED (the same convention as `kernels.scatter.rs`): the
// loaders + constants are duplicated from `kernels.rs` so the contract
// pass can compile this file standalone. The compute difference vs
// `kernels.scatter.rs` is the added pure `bucket` kernel (a value->bin
// map) — the X1' feature exercises it in INDEX position. `inc` is the
// plain increment (identical to `kernels.scatter.rs::inc`).

use std::env;
use std::fs;
use std::io::Write;

/// Input length. Mirrors `const N : usize = 256;` in
/// `prog.textbook.algo.nuc`. The doubled declaration is the v2 convention
/// per TASK-0103: kernels.rs is plain Rust compiled by the host
/// toolchain unmodified — Nucleus does not text-substitute algorithm
/// consts into kernel bodies.
const N: usize = 256;

/// Histogram bin count. Mirrors `const BINS : usize = 16;` in
/// `prog.textbook.algo.nuc`. Same TASK-0103 convention as `N`.
const BINS: usize = 16;

/// Value->bin bucketing kernel — PURE, used in INDEX position by
/// `prog.textbook.algo.nuc` (`histogram[bucket(input[i])] <-- ...`).
///
/// Maps an (in general unconstrained) input value into `[0, BINS)` with a
/// non-negative modulo. `((v % BINS) + BINS) % BINS` is the standard
/// branch-free Euclidean-remainder spelling that handles negative `v`
/// correctly (`-1` maps to `BINS-1`, not `-1`). It is deterministic and
/// side-effect-free, so it is sound in index position (PRD §6.2 purity).
///
/// For the committed pre-clipped fixture every `input[i]` is already in
/// `[0, BINS)`, so `bucket(v) == v` at runtime and the histogram is
/// bit-identical to `kernels.scatter.rs` / `kernels.rs` / `reference.bin`.
/// The kernel is written for the general (unconstrained) case regardless.
pub fn bucket(v: i32) -> i32 {
    let bins = BINS as i32;
    ((v % bins) + bins) % bins
}

/// Per-hit increment: `acc + 1`.
///
/// No value/bin arguments and no mask — the data-dependent WRITE address
/// `histogram[bucket(input[i])]` selects the bin directly, so the kernel
/// just bumps the accumulator. `wrapping_add` documents the overflow
/// contract (PRD §10.1); the committed fixture's max bin count is far
/// below `i32::MAX`.
pub fn inc(acc: i32) -> i32 {
    acc.wrapping_add(1)
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
