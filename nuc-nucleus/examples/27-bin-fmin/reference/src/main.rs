//! Reference implementation of example 27-bin-fmin (TASK-0343.02).
//!
//! Single-pass FLOAT per-bin MINIMUM: for each `(key[i], val[i])` with
//! `key[i]` in [0, BINS-1] and `val[i]` strictly positive finite f32,
//! fold `result[key[i]] = min(result[key[i]], val[i])`. The output is a
//! BINS-wide f32 LE array where `result[b]` is the minimum `val` over
//! inputs with `key == b`, or the float MIN-identity `f32::INFINITY`
//! for an EMPTY bin (no input maps to `b`).
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
//! to MUST read `f32::INFINITY` (the float MIN identity the codegen
//! pre-inits; bit pattern 0x7F800000). This reference initialises
//! `result` to `f32::INFINITY` for exactly that reason — and it is the
//! value a backend that wrongly pre-inits to 0.0 would FAIL to produce
//! (it would emit 0.0), which is how the 7-way cross-backend
//! differential bites a missed init site.
//!
//! Why float min is bit-stable here (PRD §10.1): `f32::min` is
//! order-independent for DISTINCT FINITE NON-NaN values. This reference
//! validates that every val is strictly positive AND finite (rejecting
//! NaN/Inf/0/negatives loudly), so the order-independence guarantee
//! holds and the reference's left-to-right fold equals any backend's
//! reduction order bit-for-bit. (±0.0 ties and all-NaN bins are an
//! out-of-scope documented caveat — this fixture avoids them.)
//!
//! Input format (`input.bin`): bytes [0 .. 4*N) — N i32 LE `key` words
//! each in [0, BINS-1]; bytes [4*N .. 8*N) — N f32 LE `val` words each
//! strictly positive + finite. Both validated strictly.
//!
//! Output format (`reference.bin`): bytes [0 .. 4*BINS) — BINS f32 LE
//! words, the per-bin minima (or f32::INFINITY for empty bins).
//!
//! N, BINS are fixed at the values declared in `prog.algo.nuc`. If those
//! consts change, this binary must change in the same commit (policy §3).
//!
//! Determinism rules (policy §5): no parallelism / threads / HashMap
//! iteration / Instant-derived values; strict left-to-right traversal.
//! Float min is deterministic for the validated finite-positive inputs.

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
                eprintln!("usage: bin-fmin-reference --in INPUT.bin --out OUTPUT.bin");
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

    let word_bytes = |idx: usize| -> [u8; 4] {
        let off = idx * BYTES_PER_WORD;
        [bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]
    };

    // Decode and validate the `key` stream is in [0, BINS-1]. An
    // out-of-range key would silently miss every bin in the rectangular
    // nest — the reference rejects loudly so fixture drift surfaces here.
    let mut key = [0i32; N];
    for (i, slot) in key.iter_mut().enumerate() {
        *slot = i32::from_le_bytes(word_bytes(i));
        if *slot < 0 || *slot >= BINS as i32 {
            return die(&format!(
                "key[{}] = {} is out of range [0, BINS); BINS = {}",
                i, *slot, BINS
            ));
        }
    }

    // Decode and validate the `val` stream is strictly POSITIVE AND
    // FINITE (NaN-free, Inf-free). The 0.0-init sensitivity argument (a
    // missed identity init yields min(0.0, val)=0.0 on a non-empty bin)
    // requires every val > 0; NaN/Inf would break the bit-stability
    // guarantee (the documented out-of-scope caveat) — so reject them.
    let mut val = [0f32; N];
    for (i, slot) in val.iter_mut().enumerate() {
        *slot = f32::from_le_bytes(word_bytes(N + i));
        if !slot.is_finite() || *slot <= 0.0 {
            return die(&format!(
                "val[{}] = {} must be strictly positive AND finite (the \
                 0.0-init sensitivity + bit-stability arguments require it; \
                 NaN/Inf/0/negative are rejected)",
                i, *slot
            ));
        }
    }

    // Single-pass per-bin min. `result` starts at the float MIN identity
    // `f32::INFINITY`; empty bins retain it. Strict left-to-right.
    let mut result = [f32::INFINITY; BINS];
    for i in 0..N {
        let bin = key[i] as usize;
        // Explicit `<` (not `f32::min`) so the empty-bin INFINITY stays
        // exact and the fold is an obvious total order on positives.
        if val[i] < result[bin] {
            result[bin] = val[i];
        }
    }

    // Encode and write output (f32 LE; INFINITY for empty bins).
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
    eprintln!("bin-fmin-reference: {}", msg);
    ExitCode::FAILURE
}
