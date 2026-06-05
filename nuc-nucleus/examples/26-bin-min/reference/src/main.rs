//! Reference implementation of example 26-bin-min (TASK-0343.01.02).
//!
//! Single-pass integer per-bin MINIMUM: for each `(key[i], val[i])`
//! with `key[i]` in [0, BINS-1] and `val[i]` strictly positive, fold
//! `result[key[i]] = min(result[key[i]], val[i])`. The output is a
//! BINS-wide i32 LE array where `result[b]` is the minimum `val` over
//! inputs with `key == b`, or the MIN-identity `i32::MAX` for an EMPTY
//! bin (no input maps to `b`).
//!
//! Structural independence from kernels.rs is deliberate
//! (docs/reference-impl-policy.md §2): kernels.rs does
//! `if k == bin { acc.min(v) } else { acc }` per (i, b) cell of the
//! rectangular masked nest (it visits every (input, bin) pair). This
//! reference does `result[key[i]] = min(result[key[i]], val[i])` per i
//! — a DIRECT index fold, different access pattern, same result. A
//! backend whose output matches this reference bit-for-bit is unlikely
//! to be "wrong in the same way" as the reference.
//!
//! The empty-bin identity is the load-bearing case: a bin no input maps
//! to MUST read `i32::MAX` (the MIN identity the codegen pre-inits).
//! This reference initialises `result` to `i32::MAX` for exactly that
//! reason — and it is the value a backend that wrongly pre-inits to 0
//! would FAIL to produce (it would emit 0), which is how the 7-way
//! cross-backend differential bites a missed init site.
//!
//! Input format (`input.bin`): bytes [0 .. 4*N) — N i32 LE `key` words
//! each in [0, BINS-1]; bytes [4*N .. 8*N) — N i32 LE `val` words each
//! strictly positive. Both validated strictly: out-of-range keys would
//! silently miss every bin in the rectangular nest, and a non-positive
//! val could collide with the 0-init sensitivity argument — so the
//! reference rejects both loudly.
//!
//! Output format (`reference.bin`): bytes [0 .. 4*BINS) — BINS i32 LE
//! words, the per-bin minima (or i32::MAX for empty bins).
//!
//! N, BINS are fixed at the values declared in `prog.algo.nuc`. If those
//! consts change, this binary must change in the same commit (policy §3).
//!
//! Determinism rules (policy §5): integer arithmetic only; strict
//! left-to-right traversal; no parallelism / threads / HashMap iteration
//! / Instant-derived values.

use std::env;
use std::fs;
use std::io::Write;
use std::process::ExitCode;

const N: usize = 256;
const BINS: usize = 16;
const BYTES_PER_WORD: usize = 4;
const INPUT_BYTES: usize = 2 * N * BYTES_PER_WORD; // key stream + val stream
const OUTPUT_BYTES: usize = BINS * BYTES_PER_WORD;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let mut in_path: Option<String> = None;
    let mut out_path: Option<String> = None;

    // Tiny hand-rolled arg parser. Pulling in `clap` for two flags would
    // violate the "auditable, not feature-rich" principle in policy §2.
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--in" => {
                i += 1;
                if i >= args.len() {
                    return die("--in requires a path argument");
                }
                in_path = Some(args[i].clone());
            }
            "--out" => {
                i += 1;
                if i >= args.len() {
                    return die("--out requires a path argument");
                }
                out_path = Some(args[i].clone());
            }
            "-h" | "--help" => {
                eprintln!("usage: bin-min-reference --in INPUT.bin --out OUTPUT.bin");
                return ExitCode::SUCCESS;
            }
            other => {
                return die(&format!("unknown argument: {}", other));
            }
        }
        i += 1;
    }

    let in_path = match in_path {
        Some(p) => p,
        None => return die("missing required --in PATH"),
    };
    let out_path = match out_path {
        Some(p) => p,
        None => return die("missing required --out PATH"),
    };

    // Read input. Validate size strictly: silent acceptance of
    // wrong-length files would mask fixture drift.
    let bytes = match fs::read(&in_path) {
        Ok(b) => b,
        Err(e) => return die(&format!("cannot read input {}: {}", in_path, e)),
    };
    if bytes.len() != INPUT_BYTES {
        return die(&format!(
            "input {} has {} bytes; expected exactly {} (2 * N * 4, N={})",
            in_path,
            bytes.len(),
            INPUT_BYTES,
            N
        ));
    }

    let read_word = |idx: usize| -> i32 {
        let off = idx * BYTES_PER_WORD;
        i32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
    };

    // Decode and validate the `key` stream is in [0, BINS-1]. An
    // out-of-range key would silently miss every bin in the rectangular
    // nest (kernels.rs::bin_min returns acc when no bin matches) — the
    // reference rejects loudly so fixture drift surfaces here.
    let mut key = [0i32; N];
    for (i, slot) in key.iter_mut().enumerate() {
        *slot = read_word(i);
        if *slot < 0 || *slot >= BINS as i32 {
            return die(&format!(
                "key[{}] = {} is out of range [0, BINS); BINS = {}",
                i, *slot, BINS
            ));
        }
    }

    // Decode and validate the `val` stream is strictly POSITIVE. The
    // 0-init sensitivity argument (a missed identity init yields
    // min(0, val)=0 on a non-empty bin) requires every val > 0; a
    // non-positive val would weaken the differential, so reject it.
    let mut val = [0i32; N];
    for (i, slot) in val.iter_mut().enumerate() {
        *slot = read_word(N + i);
        if *slot <= 0 {
            return die(&format!(
                "val[{}] = {} must be strictly positive (the 0-init \
                 sensitivity argument requires it)",
                i, *slot
            ));
        }
    }

    // Single-pass per-bin min. `result` starts at the MIN identity
    // `i32::MAX`; empty bins retain it. Strict left-to-right.
    let mut result = [i32::MAX; BINS];
    for i in 0..N {
        let bin = key[i] as usize;
        if val[i] < result[bin] {
            result[bin] = val[i];
        }
    }

    // Encode and write output.
    let mut out_buf = Vec::with_capacity(OUTPUT_BYTES);
    for v in &result {
        out_buf.extend_from_slice(&v.to_le_bytes());
    }
    assert_eq!(out_buf.len(), OUTPUT_BYTES);

    let mut f = match fs::File::create(&out_path) {
        Ok(f) => f,
        Err(e) => return die(&format!("cannot create output {}: {}", out_path, e)),
    };
    if let Err(e) = f.write_all(&out_buf) {
        return die(&format!("write to {} failed: {}", out_path, e));
    }

    ExitCode::SUCCESS
}

fn die(msg: &str) -> ExitCode {
    eprintln!("bin-min-reference: {}", msg);
    ExitCode::FAILURE
}
