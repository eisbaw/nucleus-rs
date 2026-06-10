//! Deterministic VGA frame-strip (640x15360 = 32 stacked VGA frames)
//! input generator for the stencil case study. Writes `H*W` flat
//! row-major i32 LE words to `--out`.
//!
//! Independence (docs/reference-impl-policy.md §2): `std` only, no
//! Nucleus dependency, no shared code with the reference oracle. The
//! generator decides the pixel values; the reference (separately)
//! decides their expected blur — keeping the two un-shared so a
//! common-mode bug cannot defeat the differential.
//!
//! DETERMINISM: the pixel value at `(y, x)` is a pure closed-form
//! function of `y` and `x` (no RNG, no clock, no allocation order
//! dependence), so re-running on any machine yields byte-identical
//! `input.bin`. The pattern is chosen to be SPATIALLY VARYING (so the
//! 3x3 blur actually changes the bytes, making the differential
//! meaningful) and BOUNDED well inside the safe range: every pixel is
//! in `[0, 65535]`, so the sum of nine taps is at most `9 * 65535 =
//! 589_815`, far below `i32::MAX` — no `wrapping_add` wraparound ever
//! occurs (consistent with kernels.rs / the reference's stated
//! safe-range contract).

use std::env;
use std::fs;
use std::io::Write;
use std::process::ExitCode;

const H: usize = 15360;
const W: usize = 640;
const N: usize = H * W;
const BYTES_PER_WORD: usize = 4;

/// Closed-form pixel value at `(y, x)`. A bounded, spatially-varying
/// pattern: two interfering ramps plus a coarse checker, masked to
/// 16 bits. Deterministic; range `[0, 65535]`.
#[inline]
fn pixel(y: usize, x: usize) -> i32 {
    // Two ramps that interfere along the diagonal, a coarse plaid, and
    // a tile-scale checker. All arithmetic in usize, masked to 16 bits
    // at the end so the value stays in [0, 65535].
    let ramp = (y.wrapping_mul(7)).wrapping_add(x.wrapping_mul(13));
    let plaid = (y / 32).wrapping_mul(x / 32).wrapping_mul(37);
    let checker = if ((y / 16) + (x / 16)) % 2 == 0 { 4096 } else { 0 };
    let v = ramp.wrapping_add(plaid).wrapping_add(checker) & 0xFFFF;
    v as i32
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let mut out_path: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                i += 1;
                if i >= args.len() {
                    return die("--out requires a path argument");
                }
                out_path = Some(args[i].clone());
            }
            "-h" | "--help" => {
                eprintln!("usage: vga-stencil-gen --out INPUT.bin");
                return ExitCode::SUCCESS;
            }
            other => return die(&format!("unknown argument: {}", other)),
        }
        i += 1;
    }
    let out_path = match out_path {
        Some(p) => p,
        None => return die("missing required --out PATH"),
    };

    let mut bytes = Vec::with_capacity(N * BYTES_PER_WORD);
    for y in 0..H {
        for x in 0..W {
            bytes.extend_from_slice(&pixel(y, x).to_le_bytes());
        }
    }
    assert_eq!(bytes.len(), N * BYTES_PER_WORD);

    let mut f = match fs::File::create(&out_path) {
        Ok(f) => f,
        Err(e) => return die(&format!("cannot create {}: {}", out_path, e)),
    };
    if let Err(e) = f.write_all(&bytes) {
        return die(&format!("write to {} failed: {}", out_path, e));
    }
    ExitCode::SUCCESS
}

fn die(msg: &str) -> ExitCode {
    eprintln!("vga-stencil-gen: {}", msg);
    ExitCode::FAILURE
}
