//! Reference implementation of example 09-producer-consumer.
//!
//! Two-stage producer/consumer pipe over an array of N i32 LE words:
//!
//!   result[n] = transform(produce(seeds[n]))
//!
//! where (independently re-implemented from `kernels.rs`, per
//! docs/reference-impl-policy.md §2):
//!
//!   produce(s)   = s * 3                      (one wrapping_mul)
//!   transform(r) = r * 7 + r                  (one wrapping_mul, one
//!                                              wrapping_add)
//!
//! Algebraically `result[n] = seeds[n] * 24` (since `(3s)*7 + 3s =
//! 24s`), but this reference NEVER folds the two stages into the
//! closed form — the point of the reference is to be a second, hand-
//! audited witness with the SAME staged shape as the algorithm and
//! kernels.rs. If a bug drops or reorders one stage in the Nucleus
//! emit, the reference must NOT silently produce the same wrong bytes.
//!
//! Independent of Nucleus per docs/reference-impl-policy.md §2:
//! depends on `std` only; does not share any code with kernels.rs or
//! the nucleus-compiler crate.
//!
//! Input format (`input.bin`):
//!   - bytes [0      ..   4*N) — array `seeds`, N i32 LE words.
//!   - Total: 4 * N = 64 bytes for N=16.
//!
//! Output format (`reference.bin`):
//!   - bytes [0      ..   4*N) — array `result`, N i32 LE words.
//!   - Total: 4 * N = 64 bytes for N=16.
//!
//! N is fixed at 16 to match `const N : usize = 16` in
//! `prog.algo.nuc`. If that const changes, this binary must change in
//! the same commit (policy §3).
//!
//! Determinism rules (policy §5):
//!   - Integer arithmetic only.
//!   - `wrapping_mul` / `wrapping_add` document the overflow contract;
//!     the committed input (seeds 1..N=16) stays well inside the i32
//!     range and never trips wrap.
//!   - No threads, no HashMap iteration, no Instant-derived values,
//!     no FMA, no f32, no SIMD reorder.

use std::env;
use std::fs;
use std::io::Write;
use std::process::ExitCode;

const N: usize = 16;
const BYTES_PER_WORD: usize = 4;
const INPUT_BYTES: usize = N * BYTES_PER_WORD;
const OUTPUT_BYTES: usize = N * BYTES_PER_WORD;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let mut in_path: Option<String> = None;
    let mut out_path: Option<String> = None;

    // Tiny hand-rolled arg parser. Pulling in `clap` for two flags
    // would violate the "auditable, not feature-rich" principle in
    // policy §2.
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
                eprintln!(
                    "usage: producer-consumer-reference --in INPUT.bin --out OUTPUT.bin"
                );
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

    // Decode the seeds array.
    let mut seeds = [0i32; N];
    for i in 0..N {
        let off = i * BYTES_PER_WORD;
        seeds[i] = i32::from_le_bytes([
            bytes[off],
            bytes[off + 1],
            bytes[off + 2],
            bytes[off + 3],
        ]);
    }

    // Compute. Two stages, written out in the SAME shape as
    // prog.algo.nuc — independent re-implementation, no shared code
    // with kernels.rs. Strict left-to-right per index.
    let mut stream = [0i32; N];
    let mut result = [0i32; N];
    for n in 0..N {
        // Stage 1: produce(seed) = seed * 3.
        stream[n] = seeds[n].wrapping_mul(3);
        // Stage 2: transform(rec) = rec * 7 + rec.
        result[n] = stream[n].wrapping_mul(7).wrapping_add(stream[n]);
    }

    // Encode and write output. Pre-size the buffer so the file length
    // is exactly OUTPUT_BYTES regardless of write granularity.
    let mut out_bytes = Vec::with_capacity(OUTPUT_BYTES);
    for v in &result {
        out_bytes.extend_from_slice(&v.to_le_bytes());
    }
    assert_eq!(out_bytes.len(), OUTPUT_BYTES);

    let mut f = match fs::File::create(&out_path) {
        Ok(f) => f,
        Err(e) => return die(&format!("cannot create output {}: {}", out_path, e)),
    };
    if let Err(e) = f.write_all(&out_bytes) {
        return die(&format!("write to {} failed: {}", out_path, e));
    }

    ExitCode::SUCCESS
}

fn die(msg: &str) -> ExitCode {
    eprintln!("producer-consumer-reference: {}", msg);
    ExitCode::FAILURE
}
