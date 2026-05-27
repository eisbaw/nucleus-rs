//! Reference implementation + fixture generator for example
//! 10-wavefront (TASK-0044.05).
//!
//! Computes the accumulated-cost wavefront a SECOND, independent way:
//! per-diagonal anti-diagonal sweep (k = 0, 1, ..., H+W-2; within each
//! diagonal iterate cells in increasing i), as opposed to the
//! Nucleus kernel's row-major sweep. Both orders produce identical
//! output because (i) the recurrence depends only on cells from
//! EARLIER diagonals (i.e. strictly smaller i+j), and (ii) min and
//! wrapping_add are deterministic and order-independent for fixed
//! operands. The point of the oracle is to be "wrong in a different
//! way if wrong at all" (docs/reference-impl-policy.md §2): if the
//! row-major Nucleus output matches this diagonal sweep bit-for-bit
//! on all 7 tier-1 backends, the recurrence is being computed
//! correctly.
//!
//! Independence (policy §2): no dependency on Nucleus, no shared code
//! with `kernels.rs`, std only, no third-party crates, no threads,
//! no HashMap, no Instant-derived values.
//!
//! Two subcommands:
//!   --gen-input PATH   write the canonical `input.bin` (H*W i32 LE
//!                      words) so there is NO python fixture step.
//!   --in PATH --out PATH
//!                      read `input.bin`, compute the accumulated
//!                      cost, write `reference.bin` (H*W i32 LE
//!                      words).
//!
//! Input/output format: bytes [0 .. 4*H*W) are H*W i32 little-endian
//! words. For the algorithm's `i32[H][W]` shape the file is
//! row-major (row i occupies words [i*W .. (i+1)*W)).
//!
//! H / W are fixed at the values declared in `prog.algo.nuc`. If
//! those consts change, this binary must change in the same commit
//! (policy §3).
//!
//! Determinism rules (policy §5): integer arithmetic only;
//! `wrapping_add` makes overflow explicit; `min` is bit-deterministic
//! by language definition.
//!
//! Input distribution (--gen-input)
//! ---------------------------------
//! Each cell `in_cost[i][j]` = ((i * 31 + j * 17 + 5) & 0x3F) + 1.
//! Range [1, 64]. The accumulated-cost matrix on 16x16 has a maximum
//! value at the (H-1, W-1) corner; with this distribution the max
//! stays below 16*16*64 = 16384, far inside i32 range. The recurrence
//! visits each cell once so there is no compounding multiplicative
//! growth — wrapping is impossible for this fixture.

use std::env;
use std::fs;
use std::io::Write;
use std::process::ExitCode;

const H: usize = 16;
const W: usize = 16;
const N: usize = H * W;
const BYTES_PER_WORD: usize = 4;
const INPUT_BYTES: usize = N * BYTES_PER_WORD;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() >= 3 && args[1] == "--gen-input" {
        return gen_input(&args[2]);
    }
    if args.len() == 5 && args[1] == "--in" && args[3] == "--out" {
        return run_reference(&args[2], &args[4]);
    }
    eprintln!(
        "usage:\n  wavefront-reference --gen-input PATH\n  wavefront-reference --in IN_PATH --out OUT_PATH"
    );
    ExitCode::FAILURE
}

fn gen_input(path: &str) -> ExitCode {
    let mut bytes = Vec::with_capacity(INPUT_BYTES);
    for i in 0..H {
        for j in 0..W {
            let v: i32 = (((i as i32) * 31 + (j as i32) * 17 + 5) & 0x3F) + 1;
            bytes.extend_from_slice(&v.to_le_bytes());
        }
    }
    let mut f = match fs::File::create(path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("--gen-input: cannot create {}: {}", path, e);
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = f.write_all(&bytes) {
        eprintln!("--gen-input: write failed: {}", e);
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn run_reference(in_path: &str, out_path: &str) -> ExitCode {
    let bytes = match fs::read(in_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("--in: cannot read {}: {}", in_path, e);
            return ExitCode::FAILURE;
        }
    };
    if bytes.len() < INPUT_BYTES {
        eprintln!(
            "--in: file {} has {} bytes; need at least {}",
            in_path,
            bytes.len(),
            INPUT_BYTES
        );
        return ExitCode::FAILURE;
    }
    let mut in_cost = vec![0i32; N];
    for k in 0..N {
        let off = k * BYTES_PER_WORD;
        in_cost[k] =
            i32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
    }

    // Diagonal sweep — deliberately a different control structure
    // from the Nucleus kernel's row-major version (policy §2
    // independence).
    let mut out = vec![0i32; N];
    let n_diagonals = H + W - 1;
    for k in 0..n_diagonals {
        // Cells on diagonal k: (i, j) with i + j == k, 0 <= i < H, 0 <= j < W.
        let i_start = if k + 1 > W { k + 1 - W } else { 0 };
        let i_end = if k + 1 < H { k + 1 } else { H };
        for i in i_start..i_end {
            let j = k - i;
            let in_v = in_cost[i * W + j];
            let v = if i == 0 && j == 0 {
                in_v
            } else if i == 0 {
                in_v.wrapping_add(out[i * W + (j - 1)])
            } else if j == 0 {
                in_v.wrapping_add(out[(i - 1) * W + j])
            } else {
                let nw = out[(i - 1) * W + (j - 1)];
                let n = out[(i - 1) * W + j];
                let w = out[i * W + (j - 1)];
                in_v.wrapping_add(nw.min(n).min(w))
            };
            out[i * W + j] = v;
        }
    }

    let mut out_bytes = Vec::with_capacity(N * BYTES_PER_WORD);
    for v in &out {
        out_bytes.extend_from_slice(&v.to_le_bytes());
    }
    let mut f = match fs::File::create(out_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("--out: cannot create {}: {}", out_path, e);
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = f.write_all(&out_bytes) {
        eprintln!("--out: write failed: {}", e);
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
