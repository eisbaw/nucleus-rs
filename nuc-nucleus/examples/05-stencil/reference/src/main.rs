//! Reference implementation of example 05-stencil.
//!
//! 3x3 box blur over a 16x16 i32 image. For each interior pixel
//! `(y, x)` in `1..H-1 x 1..W-1`, compute the truncating integer
//! division of the sum of the nine surrounding pixels by 9. Edge
//! pixels (the outermost ring) are left at zero in the output — this
//! matches the algorithm's single-assignment pattern, which writes
//! only interior pixels and leaves the boundary at the codegen's
//! zero-init default.
//!
//! Independence (docs/reference-impl-policy.md §2): no dependency on
//! Nucleus, no shared code with kernels.rs, std only. The point of
//! the reference is to be a second, hand-audited witness — if a
//! backend matches the reference bit-for-bit, we have evidence the
//! compiler is not "wrong in the same way".
//!
//! Input format (`input.bin`):
//!   - bytes [0 .. 4 * H * W) — flat row-major i32 LE words for
//!     `img_in[y][x]` at offset `(y * W + x) * 4`.
//!
//! Output format (`reference.bin`):
//!   - bytes [0 .. 4 * H * W) — flat row-major i32 LE words for
//!     `img_out`. Boundary pixels (y in {0, H-1} OR x in {0, W-1})
//!     are zero. Interior pixels are
//!     `img_in[y-1][x-1] + ... + img_in[y+1][x+1]` divided by 9
//!     (truncating integer division).
//!
//! H = W = 16; matches `const H : usize = 16;` and `const W : usize
//! = 16;` in `prog.algo.nuc`. If those constants change, this
//! binary must change in the same commit (policy §3).
//!
//! Determinism rules (policy §5):
//!   - Integer arithmetic only.
//!   - `i32::wrapping_add` documents the overflow contract; the
//!     committed fixtures stay inside the safe range.
//!   - No threads, no HashMap iteration, no Instant-derived values.

use std::env;
use std::fs;
use std::io::Write;
use std::process::ExitCode;

const H: usize = 16;
const W: usize = 16;
const N: usize = H * W;
const BYTES_PER_WORD: usize = 4;
const IMAGE_BYTES: usize = N * BYTES_PER_WORD;

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
                eprintln!("usage: stencil-reference --in INPUT.bin --out OUTPUT.bin");
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

    // Decode input image into a flat row-major Vec<i32>. Matches the
    // codegen's `Vec<i32>` convention (see kernels.rs header).
    let mut img_in = vec![0i32; N];
    for (k, slot) in img_in.iter_mut().enumerate() {
        let off = k * BYTES_PER_WORD;
        *slot = i32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
    }

    // Compute. Output is zero-initialised; only interior pixels are
    // written. This matches the algorithm's
    // `for y : 1..H-1 { for x : 1..W-1 { img_out[y][x] <-- ... } }`
    // — the boundary is single-assignment-default = zero.
    let mut img_out = vec![0i32; N];
    for y in 1..(H - 1) {
        for x in 1..(W - 1) {
            // Read the 3x3 neighbourhood. Index helper inlined for
            // auditability — a `idx(y, x) = y * W + x` would save a
            // few lines but obscure the row-major layout.
            let p0 = img_in[(y - 1) * W + (x - 1)];
            let p1 = img_in[(y - 1) * W + x];
            let p2 = img_in[(y - 1) * W + (x + 1)];
            let p3 = img_in[y * W + (x - 1)];
            let p4 = img_in[y * W + x];
            let p5 = img_in[y * W + (x + 1)];
            let p6 = img_in[(y + 1) * W + (x - 1)];
            let p7 = img_in[(y + 1) * W + x];
            let p8 = img_in[(y + 1) * W + (x + 1)];
            // Same expression as kernels.rs::blur3 — load-bearing for
            // bit-identity. Sum then truncating integer division.
            let sum = p0
                .wrapping_add(p1)
                .wrapping_add(p2)
                .wrapping_add(p3)
                .wrapping_add(p4)
                .wrapping_add(p5)
                .wrapping_add(p6)
                .wrapping_add(p7)
                .wrapping_add(p8);
            img_out[y * W + x] = sum / 9;
        }
    }

    // Encode and write output. Pre-size the buffer so the file
    // length is exactly IMAGE_BYTES regardless of write granularity.
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
    eprintln!("stencil-reference: {}", msg);
    ExitCode::FAILURE
}
