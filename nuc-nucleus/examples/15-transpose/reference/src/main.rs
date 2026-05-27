//! Reference implementation of example 15-transpose. Landed cycle 204
//! (TASK-0341.01 AC#1 language-sanity slice).
//!
//! `out[j][i] = in[i][j]` over an H x W i32 input matrix.
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
//!     in the input's `[H][W]` shape (so `in[i][j]` is at offset
//!     `(i * W + j) * 4`).
//!
//! Output format (`reference.bin`):
//!   - bytes [0 .. 4*W*H) — W*H i32 little-endian words, row-major
//!     in the output's `[W][H]` shape (so `out[j][i]` is at offset
//!     `(j * H + i) * 4`).
//!
//! H = 8, W = 16 to match `const H : usize = 8` / `const W : usize =
//! 16` in `prog.algo.nuc`. If those consts change, this binary must
//! change in the same commit (policy §3).
//!
//! Determinism rules (policy §5):
//!   - Integer values only; identity copy, no arithmetic.
//!   - No threads, no HashMap iteration, no Instant-derived values.

use std::env;
use std::fs;
use std::io::Write;
use std::process::ExitCode;

const H: usize = 8;
const W: usize = 16;
const BYTES_PER_WORD: usize = 4;
const INPUT_BYTES: usize = H * W * BYTES_PER_WORD;
const OUTPUT_BYTES: usize = W * H * BYTES_PER_WORD;

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
                    "usage: transpose-reference --in INPUT.bin --out OUTPUT.bin"
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

    // Decode `in[i][j]` from row-major `[H][W]` layout.
    let mut input = [[0i32; W]; H];
    for i in 0..H {
        for j in 0..W {
            let off = (i * W + j) * BYTES_PER_WORD;
            input[i][j] = i32::from_le_bytes([
                bytes[off],
                bytes[off + 1],
                bytes[off + 2],
                bytes[off + 3],
            ]);
        }
    }

    // Permute. `output[j][i] = input[i][j]`. Loop order is i-outer,
    // j-inner so the write to `output[j][i]` traverses output columns
    // for each fixed `j`. Determinism does not depend on order here
    // (single-assignment, no reductions), but the explicit order
    // matches what `prog.algo.nuc` declares.
    let mut output = [[0i32; H]; W];
    for i in 0..H {
        for j in 0..W {
            output[j][i] = input[i][j];
        }
    }

    // Encode `out[j][i]` into row-major `[W][H]` layout.
    let mut out_bytes = Vec::with_capacity(OUTPUT_BYTES);
    for j in 0..W {
        for i in 0..H {
            out_bytes.extend_from_slice(&output[j][i].to_le_bytes());
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
    eprintln!("transpose-reference: {}", msg);
    ExitCode::FAILURE
}
