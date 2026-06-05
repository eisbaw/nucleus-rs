//! Reference implementation of example 22-dma-pio-demo.
//!
//! Q8 fixed-point audio gain-apply over `N = 256` i32 little-endian
//! words: `out[i] = (samples[i] * gains[i]) >> 8`. The reference is
//! unaware of DMA vs PIO transport — that distinction lives entirely
//! in the Nucleus schedule and is a codegen/transport concern, not an
//! algorithmic one. The reference is the canonical sequential answer
//! the byte-exact Renode co-sim compares against.
//!
//! Independence (docs/reference-impl-policy.md §2): no dependency on
//! Nucleus, no shared code with kernels.rs, std only. The point of the
//! reference is to be a second, hand-audited witness — if the backend
//! matches the reference bit-for-bit, we have evidence the compiler is
//! not "wrong in the same way".
//!
//! Modes (run from the repo root, see Cargo.toml header for full paths):
//!   --gen-input PATH      write the canonical `input.bin`
//!                         (256 i32 LE samples ++ 256 i32 LE gains).
//!   --in PATH --out PATH  read input.bin, write reference.bin
//!                         (256 i32 LE out words).
//!
//! Input format (`input.bin`, 2048 bytes):
//!   - bytes [0       ..   4*N) — array `samples`, N i32 LE words.
//!   - bytes [4*N     .. 4*2*N) — array `gains`,   N i32 LE words.
//!
//! Output format (`reference.bin`, 1024 bytes):
//!   - bytes [0       ..   4*N) — array `out = (samples*gains)>>8`.
//!
//! Fixture pattern (`--gen-input`), deterministic, chosen so a
//! dropped/swapped element is visible and the Q8 result stays well
//! inside i32:
//!   - samples[i] = (i as i32) - 128       // signed sweep [-128, 127]
//!   - gains[i]   = 200 + (i % 100) as i32  // [200, 299], straddles
//!                                          // unity (256), so some
//!                                          // samples are attenuated
//!                                          // and some amplified.
//! |out| is bounded by 127 * 299 >> 8 = 148 (a loose cross-product
//! bound: those extremes never co-occur at the same index, so the
//! realized max is smaller), nowhere near i32 limits; `wrapping_mul`
//! documents the overflow contract regardless.
//!
//! N is fixed at 256 to match `const N : usize = 256` in
//! `prog.algo.nuc`. If that const changes, this binary must change in
//! the same commit (policy §3).
//!
//! Determinism rules (policy §5): integer arithmetic only; no threads,
//! no HashMap iteration, no Instant-derived values.

use std::env;
use std::fs;
use std::io::Write;
use std::process::ExitCode;

const N: usize = 256;
const BYTES_PER_WORD: usize = 4;
const INPUT_BYTES: usize = 2 * N * BYTES_PER_WORD;
const OUTPUT_BYTES: usize = N * BYTES_PER_WORD;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let mut in_path: Option<String> = None;
    let mut out_path: Option<String> = None;
    let mut gen_input: Option<String> = None;

    // Tiny hand-rolled arg parser. Pulling in `clap` for three flags
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
            "--gen-input" => {
                i += 1;
                if i >= args.len() {
                    return die("--gen-input requires a path argument");
                }
                gen_input = Some(args[i].clone());
            }
            "-h" | "--help" => {
                eprintln!(
                    "usage:\n  \
                     dma-pio-demo-reference --gen-input INPUT.bin\n  \
                     dma-pio-demo-reference --in INPUT.bin --out REFERENCE.bin"
                );
                return ExitCode::SUCCESS;
            }
            other => {
                return die(&format!("unknown argument: {}", other));
            }
        }
        i += 1;
    }

    if let Some(p) = gen_input {
        return gen_input_file(&p);
    }

    let in_path = match in_path {
        Some(p) => p,
        None => return die("missing required --in PATH (or use --gen-input PATH)"),
    };
    let out_path = match out_path {
        Some(p) => p,
        None => return die("missing required --out PATH"),
    };

    // Read input. Validate size strictly: silent acceptance of a
    // wrong-length file would mask fixture drift.
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

    // Decode arrays: samples in [0..N), gains in [N..2N).
    let mut samples = [0i32; N];
    let mut gains = [0i32; N];
    for i in 0..N {
        let off_s = i * BYTES_PER_WORD;
        samples[i] = i32::from_le_bytes([
            bytes[off_s],
            bytes[off_s + 1],
            bytes[off_s + 2],
            bytes[off_s + 3],
        ]);
        let off_g = (N + i) * BYTES_PER_WORD;
        gains[i] = i32::from_le_bytes([
            bytes[off_g],
            bytes[off_g + 1],
            bytes[off_g + 2],
            bytes[off_g + 3],
        ]);
    }

    // Compute. Strict left-to-right per index; Q8 fixed-point gain.
    // MUST match kernels.rs `apply_gain` EXACTLY: wrapping_mul then >>8.
    let mut out = [0i32; N];
    for i in 0..N {
        out[i] = (samples[i].wrapping_mul(gains[i])) >> 8;
    }

    // Encode and write output. Pre-size the buffer so the file length
    // is exactly OUTPUT_BYTES regardless of write granularity.
    let mut out_bytes = Vec::with_capacity(OUTPUT_BYTES);
    for v in &out {
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

/// Write the canonical `input.bin`: 256 i32 LE samples followed by
/// 256 i32 LE gains. See the module header for the pattern rationale.
fn gen_input_file(path: &str) -> ExitCode {
    let mut bytes = Vec::with_capacity(INPUT_BYTES);
    // samples first (matches load_samples consuming positions 0..N).
    for i in 0..N {
        let v: i32 = (i as i32) - 128;
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    // gains second (matches load_gains consuming positions N..2N).
    for i in 0..N {
        let v: i32 = 200 + (i % 100) as i32;
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    assert_eq!(bytes.len(), INPUT_BYTES);

    let mut f = match fs::File::create(path) {
        Ok(f) => f,
        Err(e) => return die(&format!("cannot create {}: {}", path, e)),
    };
    if let Err(e) = f.write_all(&bytes) {
        return die(&format!("write to {} failed: {}", path, e));
    }
    ExitCode::SUCCESS
}

fn die(msg: &str) -> ExitCode {
    eprintln!("dma-pio-demo-reference: {}", msg);
    ExitCode::FAILURE
}
