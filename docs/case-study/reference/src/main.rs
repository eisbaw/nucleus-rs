//! Reference oracle for the VGA frame-strip stencil production case study.
//!
//! 3x3 box blur over a 640x15360 i32 image (32 VGA frames stacked).
//! For each interior pixel
//! `(y, x)` in `1..H-1 x 1..W-1`, compute the truncating integer
//! division of the sum of the nine surrounding pixels by 9. Edge
//! pixels (the outermost ring) are left at zero — matching the
//! algorithm's single-assignment pattern, which writes only interior
//! pixels and leaves the boundary at the codegen's zero-init default.
//!
//! INDEPENDENCE (docs/reference-impl-policy.md §2): no dependency on
//! Nucleus, no shared code with the case study's kernels.rs, `std`
//! only. The point of the reference is to be a second, hand-audited
//! witness — if a backend matches the reference bit-for-bit, we have
//! evidence the compiler is not "wrong in the same way" (PRD §10.1).
//!
//! This is a deliberately RE-DERIVED implementation, not a copy of
//! kernels.rs: it uses a different loop structure (an `idx` helper and
//! a tap array) so a transcription bug here is unlikely to coincide
//! with one in the kernel. The arithmetic it must agree on — the
//! left-to-right wrapping sum and the `/9` truncating divide — is the
//! load-bearing contract; everything else is restructured.
//!
//! Input format (`input.bin`):
//!   - bytes [0 .. 4*H*W) — flat row-major i32 LE words for
//!     `img_in[y][x]` at byte offset `(y * W + x) * 4`.
//!
//! Output format (`reference.bin`):
//!   - bytes [0 .. 4*H*W) — flat row-major i32 LE words for `img_out`.
//!     Boundary pixels (y in {0, H-1} OR x in {0, W-1}) are zero.
//!
//! H = 15360, W = 640; matches `const H : usize = 15360;` / `const W :
//! usize = 640;` in prog.algo.nuc. If those constants change, this
//! binary must change in the same commit (policy §3).
//!
//! Determinism rules (policy §5): integer arithmetic only;
//! `i32::wrapping_add` documents the overflow contract; no threads, no
//! HashMap iteration, no Instant-derived values.

use std::env;
use std::fs;
use std::io::Write;
use std::process::ExitCode;

const H: usize = 15360;
const W: usize = 640;
const N: usize = H * W;
const BYTES_PER_WORD: usize = 4;
const IMAGE_BYTES: usize = N * BYTES_PER_WORD;

/// Row-major flat index. Inlined contract: `img[y][x]` lives at
/// `y * W + x` in the flat Vec (matches the codegen's flattening and
/// the file's byte order).
#[inline]
fn idx(y: usize, x: usize) -> usize {
    y * W + x
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let mut in_path: Option<String> = None;
    let mut out_path: Option<String> = None;

    // Tiny hand-rolled arg parser; pulling in `clap` for two flags
    // would violate the "auditable, not feature-rich" principle (§2).
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
                eprintln!("usage: vga-stencil-reference --in INPUT.bin --out OUTPUT.bin");
                return ExitCode::SUCCESS;
            }
            other => return die(&format!("unknown argument: {}", other)),
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

    // Read + strict size check. Silent acceptance of a wrong-length
    // file would mask fixture drift.
    let bytes = match fs::read(&in_path) {
        Ok(b) => b,
        Err(e) => return die(&format!("cannot read input {}: {}", in_path, e)),
    };
    if bytes.len() != IMAGE_BYTES {
        return die(&format!(
            "input {} has {} bytes; expected exactly {} (H*W*4, H={}, W={})",
            in_path,
            bytes.len(),
            IMAGE_BYTES,
            H,
            W
        ));
    }

    // Decode into a flat row-major Vec<i32>.
    let mut img_in = vec![0i32; N];
    for (k, slot) in img_in.iter_mut().enumerate() {
        let off = k * BYTES_PER_WORD;
        *slot = i32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
    }

    // Compute. Output is zero-initialised; only interior pixels are
    // written — matching `for y : 1..H-1 { for x : 1..W-1 { ... } }`.
    // Re-derived shape: gather the nine taps into an array, then fold.
    let mut img_out = vec![0i32; N];
    for y in 1..(H - 1) {
        for x in 1..(W - 1) {
            let taps = [
                img_in[idx(y - 1, x - 1)],
                img_in[idx(y - 1, x)],
                img_in[idx(y - 1, x + 1)],
                img_in[idx(y, x - 1)],
                img_in[idx(y, x)],
                img_in[idx(y, x + 1)],
                img_in[idx(y + 1, x - 1)],
                img_in[idx(y + 1, x)],
                img_in[idx(y + 1, x + 1)],
            ];
            // Load-bearing contract with kernels.rs::blur3: the
            // left-to-right wrapping sum followed by truncating `/9`.
            // `fold` walks taps[0..9] in order, so the accumulation
            // order matches blur3's chained `.wrapping_add`.
            let sum = taps.iter().fold(0i32, |acc, &t| acc.wrapping_add(t));
            img_out[idx(y, x)] = sum / 9;
        }
    }

    // Encode + write. Pre-size so the file is exactly IMAGE_BYTES.
    let mut out_bytes = Vec::with_capacity(IMAGE_BYTES);
    for v in &img_out {
        out_bytes.extend_from_slice(&v.to_le_bytes());
    }
    assert_eq!(out_bytes.len(), IMAGE_BYTES);

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
    eprintln!("vga-stencil-reference: {}", msg);
    ExitCode::FAILURE
}
