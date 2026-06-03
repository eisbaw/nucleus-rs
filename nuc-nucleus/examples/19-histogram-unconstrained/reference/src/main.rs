//! Reference implementation of example 19-histogram-unconstrained.
//!
//! Single-pass integer histogram over TRULY-UNCONSTRAINED input: for each
//! input value `v` (which may be negative or `>= BINS`), fold it into a
//! bin in `[0, BINS)` THROUGH the Euclidean-remainder bucket and increment
//! that bin. Output is a BINS-wide i32 LE array.
//!
//! CRITICAL: unlike 08-histogram's reference (which STRICTLY validates
//! `v in [0, BINS)` and rejects out-of-range input), this reference does
//! NOT validate the range — folding out-of-range values through the
//! modulo is THE WHOLE POINT of the example (TASK-0432 AC#2). The bucket
//! must do real work; rejecting out-of-range input would defeat the
//! demonstration.
//!
//! Structural independence from kernels.textbook.rs (policy §2): the
//! kernel spells the bucket as `((v % BINS) + BINS) % BINS` (a manual
//! branch-free non-negative modulo). This reference uses the stdlib
//! `i32::rem_euclid`, a structurally DIFFERENT computation of the same
//! mathematical Euclidean remainder. A backend matching this reference
//! bit-for-bit is therefore unlikely to be "wrong in the same way".
//!
//! Input format (`input.bin`):
//!   - bytes [0 .. 4*N) — N i32 little-endian words. Values are
//!     UNCONSTRAINED (negatives and `>= BINS` are expected and exercised).
//!     The size is still validated strictly (silent acceptance of a
//!     wrong-length file would mask fixture drift).
//!
//! Output format (`reference.bin`):
//!   - bytes [0 .. 4*BINS) — BINS i32 LE words, the histogram.
//!
//! N, BINS are fixed at the values declared in `prog.textbook.algo.nuc`.
//! If those consts change, this binary must change in the same commit
//! (policy §3).
//!
//! Determinism rules (policy §5):
//!   - Integer arithmetic only.
//!   - `i32::wrapping_add` documents overflow intent.
//!   - Strict left-to-right traversal of input.
//!   - No parallelism, no threads, no HashMap iteration, no
//!     Instant-derived values.

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
                eprintln!(
                    "usage: histogram-unconstrained-reference --in INPUT.bin --out OUTPUT.bin"
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

    // Read input. Validate SIZE strictly (wrong-length files mask fixture
    // drift) but NOT range: out-of-range values are the point.
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

    let mut input = [0i32; N];
    for (i, slot) in input.iter_mut().enumerate() {
        let off = i * BYTES_PER_WORD;
        *slot = i32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
    }

    // Single-pass histogram THROUGH the Euclidean-remainder bucket. Strict
    // left-to-right; `rem_euclid` always returns a value in `[0, BINS)`
    // for a positive modulus, so the index is always in bounds even for
    // negative or `>= BINS` input. `wrapping_add` documents overflow.
    let bins = BINS as i32;
    let mut histogram = [0i32; BINS];
    for v in input.iter() {
        let bin = v.rem_euclid(bins) as usize;
        histogram[bin] = histogram[bin].wrapping_add(1);
    }

    // Encode and write output.
    let mut out_buf = Vec::with_capacity(OUTPUT_BYTES);
    for v in &histogram {
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
    eprintln!("histogram-unconstrained-reference: {}", msg);
    ExitCode::FAILURE
}
