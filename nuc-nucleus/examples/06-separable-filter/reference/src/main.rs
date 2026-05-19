//! Reference implementation + fixture generator for example
//! 06-separable-filter (TASK-0040).
//!
//! Computes the 5x5 separable box SUM a SECOND, independent way:
//! two explicit passes that, for each output, sum exactly the five
//! clamp-to-edge taps with a small `for off in -2..=2` loop and an
//! explicit `clamp` — i.e. the textbook stencil, NOT the rectangular
//! "visit every column/row and mask" accumulator the Nucleus program
//! uses. Different algorithm, same result: if the Nucleus output
//! matches this bit-for-bit on both backends, the rectangular
//! encoding is sound (docs/reference-impl-policy.md §2).
//!
//! Independence (policy §2): no dependency on Nucleus, no shared code
//! with `kernels.rs`, std only, no third-party crates, no threads,
//! no HashMap, no Instant-derived values.
//!
//! Two subcommands:
//!   --gen-input PATH      write the canonical `input.bin` (H*W i32
//!                         LE words) so there is NO python step.
//!   --in PATH --out PATH  read `input.bin`, run the separable
//!                         filter, write `reference.bin`.
//!
//! Format: bytes [0 .. 4*H*W) are H*W i32 LE words, row-major
//! (`v[y*W + x]`). H = W = 16; if those consts change in
//! `prog.algo.nuc` this binary must change in the same commit
//! (policy §3).
//!
//! Boundary policy: CLAMP-to-edge (replicate). A tap that falls
//! outside the image is replaced by the nearest edge sample. This
//! matches `kernels.rs` exactly (the differential would be
//! meaningless otherwise) but is computed here with a deliberately
//! different control structure.
//!
//! Determinism rules (policy §5): integer arithmetic only;
//! `wrapping_add` documents overflow; strict, no parallelism. This
//! is a box SUM (no divide) — same as the algorithm — so there is no
//! rounding choice to drift.

use std::env;
use std::fs;
use std::io::Write;
use std::process::ExitCode;

const H: usize = 16;
const W: usize = 16;
const N: usize = H * W;
const RADIUS: i32 = 2;
const BYTES_PER_WORD: usize = 4;
const INPUT_BYTES: usize = N * BYTES_PER_WORD;

fn clamp_idx(v: i32, hi: i32) -> i32 {
    if v < 0 {
        0
    } else if v > hi {
        hi
    } else {
        v
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let mut in_path: Option<String> = None;
    let mut out_path: Option<String> = None;
    let mut gen_input: Option<String> = None;

    // Tiny hand-rolled arg parser. Pulling in `clap` would violate
    // the "auditable, not feature-rich" principle (policy §2).
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
                     separable-filter-reference --gen-input INPUT.bin\n  \
                     separable-filter-reference --in INPUT.bin --out REFERENCE.bin"
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

    let bytes = match fs::read(&in_path) {
        Ok(b) => b,
        Err(e) => return die(&format!("cannot read input {}: {}", in_path, e)),
    };
    if bytes.len() != INPUT_BYTES {
        return die(&format!(
            "input {} has {} bytes; expected exactly {} (H*W*4, H=W={})",
            in_path,
            bytes.len(),
            INPUT_BYTES,
            H
        ));
    }

    // Decode row-major.
    let mut img = [0i32; N];
    for (k, slot) in img.iter_mut().enumerate() {
        let off = k * BYTES_PER_WORD;
        *slot = i32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
    }

    // Pass 1 — horizontal: tmp[y][x] = sum_{off=-2..=2}
    // img[y][clamp(x+off, 0, W-1)]. Explicit tap loop (NOT the
    // visit-every-column accumulator the Nucleus program uses).
    let mut tmp = [0i32; N];
    let w_hi = W as i32 - 1;
    for y in 0..H {
        for x in 0..W {
            let mut acc = 0i32;
            for off in -RADIUS..=RADIUS {
                let cx = clamp_idx(x as i32 + off, w_hi) as usize;
                acc = acc.wrapping_add(img[y * W + cx]);
            }
            tmp[y * W + x] = acc;
        }
    }

    // Pass 2 — vertical: out[y][x] = sum_{off=-2..=2}
    // tmp[clamp(y+off, 0, H-1)][x].
    let mut out = [0i32; N];
    let h_hi = H as i32 - 1;
    for y in 0..H {
        for x in 0..W {
            let mut acc = 0i32;
            for off in -RADIUS..=RADIUS {
                let cy = clamp_idx(y as i32 + off, h_hi) as usize;
                acc = acc.wrapping_add(tmp[cy * W + x]);
            }
            out[y * W + x] = acc;
        }
    }

    // Encode and write.
    let mut bytes_out = Vec::with_capacity(N * BYTES_PER_WORD);
    for v in &out {
        bytes_out.extend_from_slice(&v.to_le_bytes());
    }
    let mut f = match fs::File::create(&out_path) {
        Ok(f) => f,
        Err(e) => return die(&format!("cannot create output {}: {}", out_path, e)),
    };
    if let Err(e) = f.write_all(&bytes_out) {
        return die(&format!("write to {} failed: {}", out_path, e));
    }

    ExitCode::SUCCESS
}

/// Write the canonical `input.bin`: H*W i32 LE words, value pattern
/// `img[y][x] = (y * 13 + x * 7) % 251 - 125`. Deterministic, varies
/// in BOTH axes (so a transposed/dropped row or column shows up in
/// the filtered output), and stays well inside i32 (values in
/// [-125, 125]; the 25-tap box sum is at most ~25 * 125 = 3125, no
/// wraparound). Row-major over `i32[H][W]` is automatic from the
/// flat 0..N walk.
fn gen_input_file(path: &str) -> ExitCode {
    let mut bytes = Vec::with_capacity(INPUT_BYTES);
    for y in 0..H {
        for x in 0..W {
            let v: i32 = ((y as i32) * 13 + (x as i32) * 7) % 251 - 125;
            bytes.extend_from_slice(&v.to_le_bytes());
        }
    }
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
    eprintln!("separable-filter-reference: {}", msg);
    ExitCode::FAILURE
}
