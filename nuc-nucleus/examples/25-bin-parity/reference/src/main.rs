//! Reference implementation of example 25-bin-parity (TASK-0343.01.01).
//!
//! Single-pass integer bin-parity: for each input value `v` in
//! [0, BINS-1], TOGGLE `parity[v]` (XOR with 1). Output is a BINS-wide
//! i32 LE array where `parity[b]` is `(count of inputs equal to b)
//! mod 2`.
//!
//! Structural independence from kernels.rs is deliberate
//! (docs/reference-impl-policy.md §2): kernels.rs does
//! `if value == bin { acc ^ 1 } else { acc }` per (i, b) cell of the
//! rectangular masked nest. This reference does `parity[value] ^= 1`
//! per i — a DIRECT index toggle, different access pattern, same
//! result. A backend whose output matches this reference bit-for-bit is
//! unlikely to be "wrong in the same way" as the reference.
//!
//! Input format (`input.bin`): bytes [0 .. 4*N) — N i32 LE words, each
//! in [0, BINS-1] (validated strictly; out-of-range would silently miss
//! every bin in the rectangular nest, so the reference rejects loudly).
//!
//! Output format (`reference.bin`): bytes [0 .. 4*BINS) — BINS i32 LE
//! words, each 0 or 1, the per-bin parity.
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
const INPUT_BYTES: usize = N * BYTES_PER_WORD;
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
                eprintln!("usage: bin-parity-reference --in INPUT.bin --out OUTPUT.bin");
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
            "input {} has {} bytes; expected exactly {} (N * 4, N={})",
            in_path,
            bytes.len(),
            INPUT_BYTES,
            N
        ));
    }

    // Decode and validate each value is in [0, BINS-1]. Out-of-range
    // values would silently miss every bin in the rectangular nest
    // (kernels.rs::bin_xor returns acc when no bin matches) — the
    // reference rejects them loudly so fixture drift surfaces here, not
    // as a silent no-toggle.
    let mut input = [0i32; N];
    for (i, slot) in input.iter_mut().enumerate() {
        let off = i * BYTES_PER_WORD;
        *slot = i32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
        if *slot < 0 || *slot >= BINS as i32 {
            return die(&format!(
                "input[{}] = {} is out of range [0, BINS); BINS = {}",
                i, *slot, BINS
            ));
        }
    }

    // Single-pass parity. Strict left-to-right; each input value toggles
    // its bin's bit (XOR with 1). The final `parity[b]` is the count of
    // inputs equal to `b`, mod 2.
    let mut parity = [0i32; BINS];
    for v in input.iter() {
        let bin = *v as usize;
        parity[bin] ^= 1;
    }

    // Encode and write output.
    let mut out_buf = Vec::with_capacity(OUTPUT_BYTES);
    for v in &parity {
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
    eprintln!("bin-parity-reference: {}", msg);
    ExitCode::FAILURE
}
