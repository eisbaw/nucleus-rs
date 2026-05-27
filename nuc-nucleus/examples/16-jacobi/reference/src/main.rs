//! Reference implementation of example 16-jacobi. Landed cycle 206
//! (TASK-0341.02 AC#1 fixed-iteration language-sanity slice).
//!
//! Five-point Jacobi iteration on a 2D H x W i32 grid with a
//! Dirichlet zero-boundary, fixed ITERS rounds:
//!
//!   field[0][y][x]  = seed[y][x]
//!   field[t][y][x]  = (field[t-1][y-1][x] + field[t-1][y+1][x]
//!                    + field[t-1][y][x-1] + field[t-1][y][x+1]) / 4
//!                    (for t >= 1, y in 1..H-1, x in 1..W-1)
//!
//! Boundary cells (y in {0, H-1} or x in {0, W-1}) stay 0 in every
//! `field[t]` — they are never written by the iteration loop. This
//! matches `prog.algo.nuc`'s Dirichlet zero-boundary semantics:
//! `for y : 1..H-1, x : 1..W-1` only writes the interior; cells outside
//! the interior are at their single-assignment default of 0.
//!
//! Independent of Nucleus per docs/reference-impl-policy.md §2:
//! depends on `std` only; does not share any code with kernels.rs or
//! the nucleus-compiler crate. The point of the reference is to be a
//! second, hand-audited witness — if a backend matches the reference
//! bit-for-bit, we have evidence the compiler is not "wrong in the
//! same way".
//!
//! Input format (`input.bin`):
//!   - bytes [0 .. 4*H*W) — H*W i32 little-endian words, row-major
//!     in the `[H][W]` shape (so `seed[y][x]` is at offset
//!     `(y * W + x) * 4`).
//!
//! Output format (`reference.bin`):
//!   - bytes [0 .. 4*H*W) — H*W i32 LE words = `field[ITERS]`, also
//!     row-major.
//!
//! H = 8, W = 8, ITERS = 4 to match `prog.algo.nuc`. If those consts
//! change, this binary must change in the same commit (policy §3).
//!
//! Determinism rules (policy §5):
//!   - Integer arithmetic only; `wrapping_add` for the 4-tap sum,
//!     truncating `/ 4` for the average.
//!   - No threads, no HashMap iteration, no Instant-derived values.

use std::env;
use std::fs;
use std::io::Write;
use std::process::ExitCode;

const H: usize = 8;
const W: usize = 8;
const ITERS: usize = 4;
const BYTES_PER_WORD: usize = 4;
const INPUT_BYTES: usize = H * W * BYTES_PER_WORD;
const OUTPUT_BYTES: usize = H * W * BYTES_PER_WORD;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let mut in_path: Option<String> = None;
    let mut out_path: Option<String> = None;

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
                    "usage: jacobi-reference --in INPUT.bin --out OUTPUT.bin"
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

    let bytes = match fs::read(&in_path) {
        Ok(b) => b,
        Err(e) => return die(&format!("cannot read input {}: {}", in_path, e)),
    };
    if bytes.len() != INPUT_BYTES {
        return die(&format!(
            "input {} has {} bytes; expected exactly {} (H*W*4, H={}, W={})",
            in_path,
            bytes.len(),
            INPUT_BYTES,
            H,
            W
        ));
    }

    // Decode the seed in row-major `[H][W]` layout.
    let mut seed = [[0i32; W]; H];
    for y in 0..H {
        for x in 0..W {
            let off = (y * W + x) * BYTES_PER_WORD;
            seed[y][x] = i32::from_le_bytes([
                bytes[off],
                bytes[off + 1],
                bytes[off + 2],
                bytes[off + 3],
            ]);
        }
    }

    // Allocate field[0..=ITERS]. Boundary cells stay at 0 throughout
    // (Dirichlet BC). field[0] gets the seed values written into its
    // interior; the algorithm's single-assignment semantics mean
    // field[0][boundary] stays 0 (the seed[boundary] cells are never
    // copied — only the interior is written by the for-nest in
    // `prog.algo.nuc`).
    let mut field = vec![[[0i32; W]; H]; ITERS + 1];
    for y in 1..H - 1 {
        for x in 1..W - 1 {
            field[0][y][x] = seed[y][x];
        }
    }

    // Iterate. The y, x walks match `prog.algo.nuc` exactly (1..H-1,
    // 1..W-1). Order of the inner accumulation matches kernels.rs
    // `jacobi5_or_seed`: north + south + east + west, then / 4.
    for t in 1..=ITERS {
        for y in 1..H - 1 {
            for x in 1..W - 1 {
                let pn = field[t - 1][y - 1][x];
                let ps = field[t - 1][y + 1][x];
                let pe = field[t - 1][y][x - 1];
                let pw = field[t - 1][y][x + 1];
                let sum = pn.wrapping_add(ps).wrapping_add(pe).wrapping_add(pw);
                field[t][y][x] = sum / 4;
            }
        }
    }

    // Emit field[ITERS] in row-major `[H][W]` order (full grid,
    // including the 0-valued boundary cells).
    let mut out_bytes = Vec::with_capacity(OUTPUT_BYTES);
    for y in 0..H {
        for x in 0..W {
            out_bytes.extend_from_slice(&field[ITERS][y][x].to_le_bytes());
        }
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
    eprintln!("jacobi-reference: {}", msg);
    ExitCode::FAILURE
}
